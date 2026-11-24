package dev.frazse.terra

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import androidx.annotation.Keep
import java.util.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@Keep
@SuppressLint("MissingPermission")
object BleManager {
  private const val TAG = "TerraBleManager"
  private val CCCD_UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

  private var context: Context? = null
  private var adapter: BluetoothAdapter? = null
  private var scanner: BluetoothLeScanner? = null

  private val scannedDevices = ConcurrentHashMap<String, DiscoveredDevice>()
  private val connectedGatts = ConcurrentHashMap<String, BluetoothGatt>()

  data class ReadRequest(val latch: CountDownLatch, var resultB64: String = "", var error: String? = null)
  data class WriteRequest(val latch: CountDownLatch, var error: String? = null)

  private val pendingReads = ConcurrentHashMap<String, ReadRequest>()
  private val pendingWrites = ConcurrentHashMap<String, WriteRequest>()
  private val subscribedChars = ConcurrentHashMap.newKeySet<String>()

  @Volatile
  private var isScanning = false
  private val scanLock = Any()
  private val mainHandler = Handler(Looper.getMainLooper())

  private data class QueuedGattOp(val id: Long, val action: Runnable)

  private class DeviceGattQueue {
    val queue = ArrayDeque<QueuedGattOp>()
    var activeOp: QueuedGattOp? = null
    var watchdog: Runnable? = null
  }

  private val gattQueues = ConcurrentHashMap<String, DeviceGattQueue>()
  private val nextOpId = java.util.concurrent.atomic.AtomicLong(1L)

  private fun queueFor(address: String): DeviceGattQueue =
    gattQueues.computeIfAbsent(address.lowercase()) { DeviceGattQueue() }

  private fun enqueueGattWrite(address: String, action: Runnable): Long {
    val op = QueuedGattOp(nextOpId.getAndIncrement(), action)
    val q = queueFor(address)
    synchronized(q) {
      q.queue.addLast(op)
      processNextGattWriteLocked(address, q)
    }
    return op.id
  }

  private fun processNextGattWriteLocked(address: String, q: DeviceGattQueue) {
    if (q.activeOp != null) return
    val next = q.queue.pollFirst() ?: return
    q.activeOp = next
    val watchdog = Runnable {
      Log.w(TAG, "GATT op #${next.id} timed out for $address — advancing queue")
      onGattWriteCompleted(address, expectedOpId = next.id)
    }
    q.watchdog = watchdog
    mainHandler.postDelayed(watchdog, 5500L)
    mainHandler.post(next.action)
  }

  private fun onGattWriteCompleted(address: String, expectedOpId: Long? = null) {
    val q = gattQueues[address.lowercase()] ?: return
    synchronized(q) {
      val cur = q.activeOp ?: return
      if (expectedOpId != null && cur.id != expectedOpId) return
      q.watchdog?.let { mainHandler.removeCallbacks(it) }
      q.watchdog = null
      q.activeOp = null
      processNextGattWriteLocked(address, q)
    }
  }

  private fun clearDeviceState(address: String, reason: String) {
    val key = address.lowercase()
    gattQueues.remove(key)?.let { q ->
      synchronized(q) {
        q.watchdog?.let { mainHandler.removeCallbacks(it) }
        q.watchdog = null
        q.activeOp = null
        q.queue.clear()
      }
    }
    val prefix = "$key:"
    val descPrefix = "desc_$key:"
    val readIt = pendingReads.entries.iterator()
    while (readIt.hasNext()) {
      val entry = readIt.next()
      if (entry.key.startsWith(prefix)) {
        readIt.remove()
        entry.value.error = reason
        entry.value.latch.countDown()
      }
    }
    val writeIt = pendingWrites.entries.iterator()
    while (writeIt.hasNext()) {
      val entry = writeIt.next()
      if (entry.key.startsWith(prefix) || entry.key.startsWith(descPrefix)) {
        writeIt.remove()
        entry.value.error = reason
        entry.value.latch.countDown()
      }
    }
    subscribedChars.removeIf { it.startsWith(prefix) }
  }

  private fun syncForegroundService() {
    TerraBleService.syncDeviceCount(context, connectedGatts.size)
  }

  data class DiscoveredDevice(
    val device: BluetoothDevice,
    var rssi: Int,
    var name: String?,
    var scanRecord: ScanRecord?,
    var lastSeenMs: Long = SystemClock.elapsedRealtime(),
    val serviceUuids: MutableSet<String> = Collections.synchronizedSet(LinkedHashSet())
  )

  @JvmStatic
  fun init(ctx: Context) {
    this.context = ctx.applicationContext
    val manager = ctx.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
    this.adapter = manager?.adapter
    Log.i(TAG, "BleManager initialized. Adapter available: ${adapter != null}")
    try {
      System.loadLibrary("terra")
      registerClassJni()
    } catch (e: Exception) {
      Log.e(TAG, "Failed to register JNI class", e)
    }
  }

  @JvmStatic
  fun startScan(): Boolean {
    synchronized(scanLock) {
      if (isScanning) {
        return true
      }
      val bAdapter = adapter ?: run {
        Log.e(TAG, "startScan failed: adapter is null")
        return false
      }
      if (!bAdapter.isEnabled) {
        Log.e(TAG, "startScan failed: Bluetooth adapter is disabled")
        return false
      }
      val bScanner = bAdapter.bluetoothLeScanner ?: run {
        Log.e(TAG, "startScan failed: bluetoothLeScanner is null")
        return false
      }

      // Prune stale advertisements (>30s) rather than wiping everything so recently seen
      // peripherals stay visible across brief scan pauses during GATT connect().
      val now = SystemClock.elapsedRealtime()
      scannedDevices.entries.removeIf { (addr, disc) ->
        !connectedGatts.containsKey(addr) && (now - disc.lastSeenMs) > 30_000L
      }

      try {
        val settings = ScanSettings.Builder()
          .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
          .build()
        bScanner.startScan(null, settings, scanCallback)
        isScanning = true
        Log.i(TAG, "BLE scan started successfully with LOW_LATENCY mode")
        return true
      } catch (e: Exception) {
        Log.e(TAG, "Failed to start BLE scan with LOW_LATENCY, trying BALANCED", e)
        try {
          val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .build()
          bScanner.startScan(null, settings, scanCallback)
          isScanning = true
          Log.i(TAG, "BLE scan started successfully with BALANCED mode")
          return true
        } catch (e2: Exception) {
          Log.e(TAG, "Failed to start BLE scan", e2)
          return false
        }
      }
    }
  }

  @JvmStatic
  fun stopScan() {
    synchronized(scanLock) {
      try {
        if (isScanning) {
          adapter?.bluetoothLeScanner?.stopScan(scanCallback)
        }
      } catch (e: Exception) {
        Log.e(TAG, "Failed to stop BLE scan", e)
      } finally {
        if (isScanning) {
          isScanning = false
          Log.i(TAG, "BLE scan stopped")
        }
      }
    }
  }

  private val scanCallback = object : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      val dev = result.device ?: return
      val address = dev.address ?: return
      val name = try { result.scanRecord?.deviceName ?: dev.name } catch (_: Exception) { null }
      val now = SystemClock.elapsedRealtime()
      val existing = scannedDevices[address]
      if (existing != null) {
        existing.rssi = result.rssi
        existing.lastSeenMs = now
        if (!name.isNullOrEmpty()) {
          existing.name = name
        }
        result.scanRecord?.let { rec ->
          rec.serviceUuids?.forEach { existing.serviceUuids.add(it.uuid.toString().lowercase()) }
          if (existing.scanRecord == null || (rec.serviceUuids?.isNotEmpty() == true)) {
            existing.scanRecord = rec
          }
        }
      } else {
        val disc = DiscoveredDevice(dev, result.rssi, name, result.scanRecord, now)
        result.scanRecord?.serviceUuids?.forEach { disc.serviceUuids.add(it.uuid.toString().lowercase()) }
        scannedDevices[address] = disc
        Log.i(TAG, "onScanResult (new): addr=$address name=$name rssi=${result.rssi} svcs=${disc.serviceUuids}")
      }
    }

    override fun onBatchScanResults(results: MutableList<ScanResult>) {
      for (r in results) {
        onScanResult(ScanSettings.CALLBACK_TYPE_ALL_MATCHES, r)
      }
    }

    override fun onScanFailed(errorCode: Int) {
      Log.e(TAG, "Scan failed with error code: $errorCode")
      isScanning = false
    }
  }

  @JvmStatic
  fun getCandidatesJson(): String {
    val array = JSONArray()
    for ((address, disc) in scannedDevices) {
      val obj = JSONObject()
      obj.put("id", address)
      obj.put("name", disc.name ?: address)
      obj.put("rssi", disc.rssi)
      obj.put("kind", inferKind(disc))
      obj.put("remembered", false)
      array.put(obj)
    }
    return array.toString()
  }

  @JvmStatic
  fun getDiscoveredDevicesJson(): String {
    val array = JSONArray()
    val seen = HashSet<String>()
    val now = SystemClock.elapsedRealtime()
    for ((address, disc) in scannedDevices) {
      val isConn = connectedGatts.containsKey(address)
      if (!isConn && (now - disc.lastSeenMs) > 30_000L) continue
      seen.add(address)
      val obj = JSONObject()
      obj.put("id", address)
      if (!disc.name.isNullOrEmpty()) {
        obj.put("name", disc.name)
      }
      obj.put("rssi", disc.rssi)
      val svcSet = LinkedHashSet<String>()
      synchronized(disc.serviceUuids) {
        svcSet.addAll(disc.serviceUuids)
      }
      disc.scanRecord?.serviceUuids?.forEach {
        svcSet.add(it.uuid.toString().lowercase())
      }
      connectedGatts[address]?.services?.forEach {
        svcSet.add(it.uuid.toString().lowercase())
      }
      val svcs = JSONArray()
      svcSet.forEach { svcs.put(it) }
      obj.put("services", svcs)
      array.put(obj)
    }
    // Connected devices stop advertising and may have aged out of `scannedDevices` —
    // always include them here so native BLE & auto-pair see all connected devices.
    for ((address, gatt) in connectedGatts) {
      if (seen.contains(address)) continue
      val obj = JSONObject()
      obj.put("id", address)
      val name = try { gatt.device?.name } catch (_: Exception) { null }
      if (!name.isNullOrEmpty()) {
        obj.put("name", name)
      }
      obj.put("rssi", -50)
      val svcs = JSONArray()
      gatt.services?.forEach {
        svcs.put(it.uuid.toString().lowercase())
      }
      obj.put("services", svcs)
      array.put(obj)
    }
    return array.toString()
  }

  @JvmStatic
  fun isConnected(address: String): Boolean {
    val gatt = connectedGatts[address] ?: return false
    return gatt.services != null && gatt.services.isNotEmpty()
  }

  private fun inferKind(disc: DiscoveredDevice): String {
    val services = disc.scanRecord?.serviceUuids ?: return "Device"
    for (parcelUuid in services) {
      val s = parcelUuid.uuid.toString().lowercase()
      if (s.contains("1826") || s.contains("6e40fec1")) return "Trainer"
      if (s.contains("1818")) return "Power meter"
      if (s.contains("1816")) return "Speed / cadence"
      if (s.contains("180d")) return "Heart rate"
    }
    return "Device"
  }

  @JvmStatic
  fun connect(address: String): String {
    val ctx = context ?: return "Error: Context not initialized"
    val bAdapter = adapter ?: return "Error: Bluetooth adapter unavailable"

    val existing = connectedGatts[address]
    if (existing != null) {
      val servicesJson = buildServicesJson(existing)
      if (servicesJson.isNotEmpty() && servicesJson != "[]") {
        Log.i(TAG, "Reusing existing GATT connection to $address")
        return servicesJson
      }
      connectedGatts.remove(address)
      try { existing.close() } catch (_: Exception) {}
    }

    stopScan()
    try { Thread.sleep(300) } catch (_: Exception) {}

    val device = try {
      bAdapter.getRemoteDevice(address)
    } catch (e: Exception) {
      return "Error: Invalid device address $address"
    }

    var lastError = "Connection failed"
    for (attempt in 1..3) {
      Log.i(TAG, "Attempting GATT connection to $address (attempt $attempt/3)...")
      val latch = CountDownLatch(1)
      var servicesJson = ""
      var connectError: String? = null
      var attemptGatt: BluetoothGatt? = null

      val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
          Log.i(TAG, "onConnectionStateChange: status=$status, newState=$newState for $address")
          if (status == BluetoothGatt.GATT_SUCCESS && newState == BluetoothProfile.STATE_CONNECTED) {
            Log.i(TAG, "GATT connected to $address, discovering services...")
            mainHandler.postDelayed({
              if (!gatt.discoverServices()) {
                connectError = "discoverServices returned false"
                latch.countDown()
              }
            }, 200)
          } else if (newState == BluetoothProfile.STATE_DISCONNECTED || status != BluetoothGatt.GATT_SUCCESS) {
            Log.i(TAG, "GATT disconnected or failed for $address (status=$status, newState=$newState)")
            val removed = connectedGatts.remove(address, gatt)
            clearDeviceState(address, "Device disconnected (status=$status)")
            try { gatt.close() } catch (_: Exception) {}
            if (removed) {
              syncForegroundService()
              try { notifyDisconnectedJni(address) } catch (_: Exception) {}
            }
            if (latch.count > 0) {
              connectError = "Connection failed with status $status"
              latch.countDown()
            }
          }
        }

        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
          if (latch.count == 0L) {
            // Connection attempt already timed out; close this late GATT instance.
            try { gatt.disconnect(); gatt.close() } catch (_: Exception) {}
            return
          }
          if (status == BluetoothGatt.GATT_SUCCESS) {
            connectedGatts[address] = gatt
            syncForegroundService()
            servicesJson = buildServicesJson(gatt)
            Log.i(TAG, "GATT services discovered for $address")
          } else {
            connectError = "Service discovery failed with status $status"
          }
          latch.countDown()
        }

        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
          val req = pendingReads.remove(opKey(address, characteristic.uuid))
          if (req != null) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
              val bytes = characteristic.value ?: byteArrayOf()
              req.resultB64 = Base64.encodeToString(bytes, Base64.NO_WRAP)
            } else {
              req.error = "Read failed with status $status"
            }
            req.latch.countDown()
          }
        }

        @Suppress("DEPRECATION")
        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
          val req = pendingReads.remove(opKey(address, characteristic.uuid))
          if (req != null) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
              req.resultB64 = Base64.encodeToString(value, Base64.NO_WRAP)
            } else {
              req.error = "Read failed with status $status"
            }
            req.latch.countDown()
          }
        }

        override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
          val req = pendingWrites.remove(opKey(address, characteristic.uuid))
          if (req != null) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
              req.error = "Write failed with status $status"
            }
            req.latch.countDown()
          }
          onGattWriteCompleted(address)
        }

        override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
          val req = pendingWrites.remove("desc_${opKey(address, descriptor.characteristic.uuid)}")
          if (req != null) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
              req.error = "Descriptor write failed with status $status"
            }
            req.latch.countDown()
          }
          onGattWriteCompleted(address)
        }

        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
          val bytes = characteristic.value ?: return
          val b64 = Base64.encodeToString(bytes, Base64.NO_WRAP)
          notifyValueJni(address, characteristic.uuid.toString(), b64)
        }

        @Suppress("DEPRECATION")
        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
          val b64 = Base64.encodeToString(value, Base64.NO_WRAP)
          notifyValueJni(address, characteristic.uuid.toString(), b64)
        }
      }

      mainHandler.post {
        attemptGatt = device.connectGatt(ctx, false, callback, BluetoothDevice.TRANSPORT_LE)
      }

      val ok = latch.await(12, TimeUnit.SECONDS)
      if (ok && connectError == null && servicesJson.isNotEmpty()) {
        return servicesJson
      }
      attemptGatt?.let { g ->
        if (connectedGatts[address] !== g) {
          try { g.disconnect(); g.close() } catch (_: Exception) {}
        }
      }
      lastError = connectError ?: if (!ok) "Connection timed out" else "No services found"
      Log.w(TAG, "Attempt $attempt failed: $lastError")
      // Status 147 (GATT_CONN_TIMEOUT) or latch timeout means the peripheral didn't respond
      // at all (powered off / out of range); don't stall for 2 more 10-12s retries.
      if (!ok || lastError.contains("status 147")) {
        break
      }
      if (attempt < 3) {
        try { Thread.sleep(500) } catch (_: Exception) {}
      }
    }

    return "Error: $lastError"
  }

  private fun buildServicesJson(gatt: BluetoothGatt): String {
    val servicesArray = JSONArray()
    for (service in gatt.services) {
      val sObj = JSONObject()
      sObj.put("uuid", service.uuid.toString())
      val charsArray = JSONArray()
      for (charac in service.characteristics) {
        val cObj = JSONObject()
        val props = charac.properties
        cObj.put("uuid", charac.uuid.toString())
        cObj.put("read", (props and BluetoothGattCharacteristic.PROPERTY_READ) != 0)
        cObj.put("write", (props and BluetoothGattCharacteristic.PROPERTY_WRITE) != 0)
        cObj.put("writeWithoutResponse", (props and BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE) != 0)
        cObj.put("notify", (props and BluetoothGattCharacteristic.PROPERTY_NOTIFY) != 0)
        cObj.put("indicate", (props and BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0)
        charsArray.put(cObj)
      }
      sObj.put("characteristics", charsArray)
      servicesArray.put(sObj)
    }
    return servicesArray.toString()
  }

  @JvmStatic
  fun disconnect(address: String) {
    val gatt = connectedGatts.remove(address)
    clearDeviceState(address, "Device disconnected")
    syncForegroundService()
    if (gatt == null) return
    try {
      gatt.disconnect()
      gatt.close()
    } catch (e: Exception) {
      Log.e(TAG, "Error disconnecting GATT $address", e)
    }
  }

  private fun opKey(address: String, uuid: UUID): String =
    "${address.lowercase()}:${uuid.toString().lowercase()}"

  private fun parseUuid(str: String): UUID {
    val s = str.trim()
    return if (s.length == 4) {
      UUID.fromString("0000${s.lowercase()}-0000-1000-8000-00805f9b34fb")
    } else if (s.length == 8) {
      UUID.fromString("${s.lowercase()}-0000-1000-8000-00805f9b34fb")
    } else {
      UUID.fromString(s)
    }
  }

  @JvmStatic
  fun readCharacteristic(address: String, serviceUuidStr: String, charUuidStr: String): String {
    val gatt = connectedGatts[address] ?: return "Error: Device $address not connected"
    val serviceUuid = parseUuid(serviceUuidStr)
    val charUuid = parseUuid(charUuidStr)
    val service = gatt.getService(serviceUuid) ?: return "Error: Service $serviceUuidStr not found"
    val charac = service.getCharacteristic(charUuid) ?: return "Error: Characteristic $charUuidStr not found"

    val reqKey = opKey(address, charUuid)
    val req = ReadRequest(CountDownLatch(1))
    pendingReads[reqKey] = req

    mainHandler.post {
      if (!gatt.readCharacteristic(charac)) {
        pendingReads.remove(reqKey)
        req.error = "readCharacteristic returned false"
        req.latch.countDown()
      }
    }

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingReads.remove(reqKey)
    if (!ok) return "Error: Read timed out"
    if (req.error != null) return "Error: ${req.error}"
    return req.resultB64
  }

  @JvmStatic
  fun writeCharacteristic(address: String, serviceUuidStr: String, charUuidStr: String, b64Value: String, withResponse: Boolean): String {
    val gatt = connectedGatts[address] ?: return "Error: Device $address not connected"
    val serviceUuid = parseUuid(serviceUuidStr)
    val charUuid = parseUuid(charUuidStr)
    val service = gatt.getService(serviceUuid) ?: return "Error: Service $serviceUuidStr not found"
    val charac = service.getCharacteristic(charUuid) ?: return "Error: Characteristic $charUuidStr not found"

    val bytes = Base64.decode(b64Value, Base64.NO_WRAP)
    val reqKey = opKey(address, charUuid)

    if (!withResponse) {
      enqueueGattWrite(address, Runnable {
        var started = false
        try {
          if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val resCode = gatt.writeCharacteristic(charac, bytes, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE)
            started = resCode == BluetoothStatusCodes.SUCCESS
            if (!started) {
              Log.w(TAG, "writeCharacteristic withoutResponse (API33) address=$address char=$charUuidStr code=$resCode")
            }
          } else {
            charac.value = bytes
            charac.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            @Suppress("DEPRECATION")
            started = gatt.writeCharacteristic(charac)
            if (!started) {
              Log.w(TAG, "writeCharacteristic withoutResponse address=$address char=$charUuidStr ok=false")
            }
          }
        } catch (e: Exception) {
          Log.e(TAG, "Error in writeCharacteristic withoutResponse for $charUuidStr", e)
        } finally {
          // On Android, a successfully initiated WRITE_TYPE_NO_RESPONSE invokes onCharacteristicWrite;
          // only advance immediately if initiation failed (watchdog covers any dropped callback).
          if (!started) onGattWriteCompleted(address)
        }
      })
      return "OK"
    }

    val req = WriteRequest(CountDownLatch(1))
    pendingWrites[reqKey] = req

    val opId = enqueueGattWrite(address, Runnable {
      var okInitiated = false
      try {
        okInitiated = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
          val resCode = gatt.writeCharacteristic(charac, bytes, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT)
          resCode == BluetoothStatusCodes.SUCCESS
        } else {
          charac.value = bytes
          charac.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
          @Suppress("DEPRECATION")
          gatt.writeCharacteristic(charac)
        }
      } catch (e: Exception) {
        Log.e(TAG, "Error in writeCharacteristic for $charUuidStr", e)
        req.error = e.message
      }

      if (!okInitiated) {
        pendingWrites.remove(reqKey)
        if (req.error == null) req.error = "writeCharacteristic returned false"
        req.latch.countDown()
        onGattWriteCompleted(address)
      }
    })

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingWrites.remove(reqKey)
    if (!ok) {
      onGattWriteCompleted(address, expectedOpId = opId)
      return "Error: Write timed out"
    }
    if (req.error != null) return "Error: ${req.error}"
    return "OK"
  }

  @JvmStatic
  fun subscribeCharacteristic(address: String, serviceUuidStr: String, charUuidStr: String, enable: Boolean): String {
    val gatt = connectedGatts[address] ?: return "Error: Device $address not connected"
    val serviceUuid = parseUuid(serviceUuidStr)
    val charUuid = parseUuid(charUuidStr)
    val service = gatt.getService(serviceUuid) ?: return "Error: Service $serviceUuidStr not found"
    val charac = service.getCharacteristic(charUuid) ?: return "Error: Characteristic $charUuidStr not found"
    val charKey = opKey(address, charUuid)

    val descriptor = charac.getDescriptor(CCCD_UUID)
    if (descriptor == null) {
      mainHandler.post {
        val okSet = gatt.setCharacteristicNotification(charac, enable)
        Log.i(TAG, "setCharacteristicNotification (no CCCD) address=$address char=$charUuidStr enable=$enable okSet=$okSet")
      }
      if (enable) subscribedChars.add(charKey) else subscribedChars.remove(charKey)
      return "OK"
    }

    val hasNotify = (charac.properties and BluetoothGattCharacteristic.PROPERTY_NOTIFY) != 0
    val hasIndicate = (charac.properties and BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0
    val useIndicate = !hasNotify && hasIndicate
    val valBytes = if (enable) {
      if (useIndicate) BluetoothGattDescriptor.ENABLE_INDICATION_VALUE else BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
    } else {
      BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
    }

    val descKey = "desc_$charKey"
    val req = WriteRequest(CountDownLatch(1))
    pendingWrites[descKey] = req

    val opId = enqueueGattWrite(address, Runnable {
      try {
        val okSet = gatt.setCharacteristicNotification(charac, enable)
        Log.i(TAG, "setCharacteristicNotification address=$address char=$charUuidStr enable=$enable okSet=$okSet")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
          descriptor.value = valBytes
          val resCode = gatt.writeDescriptor(descriptor, valBytes)
          Log.i(TAG, "subscribeCharacteristic (API33) address=$address char=$charUuidStr enable=$enable useIndicate=$useIndicate resCode=$resCode")
          if (resCode != BluetoothStatusCodes.SUCCESS) {
            pendingWrites.remove(descKey)
            req.error = "writeDescriptor returned code $resCode"
            req.latch.countDown()
            onGattWriteCompleted(address)
          }
        } else {
          descriptor.value = valBytes
          @Suppress("DEPRECATION")
          val ok = gatt.writeDescriptor(descriptor)
          Log.i(TAG, "subscribeCharacteristic address=$address char=$charUuidStr enable=$enable useIndicate=$useIndicate ok=$ok")
          if (!ok) {
            pendingWrites.remove(descKey)
            req.error = "writeDescriptor returned false"
            req.latch.countDown()
            onGattWriteCompleted(address)
          }
        }
      } catch (e: Exception) {
        Log.e(TAG, "Error in writeDescriptor for $charUuidStr", e)
        pendingWrites.remove(descKey)
        req.error = e.message
        req.latch.countDown()
        onGattWriteCompleted(address)
      }
    })

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingWrites.remove(descKey)
    if (!ok) {
      onGattWriteCompleted(address, expectedOpId = opId)
      return "Error: Subscribe timed out"
    }
    if (req.error != null) return "Error: ${req.error}"
    if (enable) subscribedChars.add(charKey) else subscribedChars.remove(charKey)
    return "OK"
  }

  @JvmStatic
  fun unsubscribeAll(address: String): String {
    val gatt = connectedGatts[address] ?: return "OK"
    val prefix = "${address.lowercase()}:"
    val activeKeys = subscribedChars.filter { it.startsWith(prefix) }.toSet()
    if (activeKeys.isEmpty()) return "OK"
    try {
      for (service in gatt.services) {
        for (charac in service.characteristics) {
          val charKey = opKey(address, charac.uuid)
          if (!activeKeys.contains(charKey)) continue
          subscribedChars.remove(charKey)
          val descriptor = charac.getDescriptor(CCCD_UUID)
          if (descriptor == null) {
            mainHandler.post { gatt.setCharacteristicNotification(charac, false) }
            continue
          }
          val valBytes = BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
          enqueueGattWrite(address, Runnable {
            var started = false
            try {
              gatt.setCharacteristicNotification(charac, false)
              if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                started = gatt.writeDescriptor(descriptor, valBytes) == BluetoothStatusCodes.SUCCESS
              } else {
                descriptor.value = valBytes
                @Suppress("DEPRECATION")
                started = gatt.writeDescriptor(descriptor)
              }
            } catch (e: Exception) {
              Log.e(TAG, "Error unsubscribing ${charac.uuid} for $address", e)
            } finally {
              if (!started) onGattWriteCompleted(address)
            }
          })
        }
      }
    } catch (e: Exception) {
      Log.e(TAG, "Error unsubscribing all for $address", e)
    }
    return "OK"
  }

  @JvmStatic
  private external fun registerClassJni()
  @JvmStatic
  private external fun notifyValueJni(address: String, characteristicUuid: String, b64Value: String)
  @JvmStatic
  private external fun notifyDisconnectedJni(address: String)
}
