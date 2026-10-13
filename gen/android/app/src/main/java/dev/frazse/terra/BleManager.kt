package dev.frazse.terra

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
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

  private var isScanning = false
  private val mainHandler = Handler(Looper.getMainLooper())

  private val gattWriteQueue = ArrayDeque<Runnable>()
  private var isWritePending = false

  private fun enqueueGattWrite(action: Runnable) {
    synchronized(gattWriteQueue) {
      gattWriteQueue.addLast(action)
      processNextGattWrite()
    }
  }

  private fun processNextGattWrite() {
    synchronized(gattWriteQueue) {
      if (isWritePending) return
      val next = gattWriteQueue.pollFirst() ?: return
      isWritePending = true
      mainHandler.post(next)
    }
  }

  private fun onGattWriteCompleted() {
    synchronized(gattWriteQueue) {
      isWritePending = false
      processNextGattWrite()
    }
  }

  data class DiscoveredDevice(
    val device: BluetoothDevice,
    var rssi: Int,
    var name: String?,
    var scanRecord: ScanRecord?
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

    stopScan()
    scannedDevices.clear()

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

  @JvmStatic
  fun stopScan() {
    try {
      if (isScanning) {
        adapter?.bluetoothLeScanner?.stopScan(scanCallback)
      }
    } catch (e: Exception) {
      Log.e(TAG, "Failed to stop BLE scan", e)
    } finally {
      isScanning = false
      Log.i(TAG, "BLE scan stopped")
    }
  }

  private val scanCallback = object : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      val dev = result.device ?: return
      val address = dev.address ?: return
      val name = try { result.scanRecord?.deviceName ?: dev.name } catch (_: Exception) { null }
      Log.i(TAG, "onScanResult: addr=$address name=$name rssi=${result.rssi}")
      val existing = scannedDevices[address]
      if (existing != null) {
        existing.rssi = result.rssi
        if (!name.isNullOrEmpty()) {
          existing.name = name
        }
        if (result.scanRecord != null) {
          if (existing.scanRecord == null || (result.scanRecord?.serviceUuids?.isNotEmpty() == true)) {
            existing.scanRecord = result.scanRecord
          }
        }
      } else {
        scannedDevices[address] = DiscoveredDevice(dev, result.rssi, name, result.scanRecord)
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
    for ((address, disc) in scannedDevices) {
      val obj = JSONObject()
      obj.put("id", address)
      obj.put("name", disc.name ?: address)
      obj.put("rssi", disc.rssi)
      val svcs = JSONArray()
      disc.scanRecord?.serviceUuids?.forEach {
        svcs.put(it.uuid.toString().lowercase())
      }
      obj.put("services", svcs)
      array.put(obj)
    }
    val res = array.toString()
    Log.i(TAG, "getDiscoveredDevicesJson (count=${scannedDevices.size}): $res")
    return res
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

    stopScan()
    try { Thread.sleep(300) } catch (_: Exception) {}

    val device = try {
      bAdapter.getRemoteDevice(address)
    } catch (e: Exception) {
      return "Error: Invalid device address $address"
    }

    var existing = connectedGatts[address]
    if (existing != null) {
      val servicesJson = buildServicesJson(existing)
      if (servicesJson.isNotEmpty() && servicesJson != "[]") {
        Log.i(TAG, "Reusing existing GATT connection to $address")
        return servicesJson
      }
      connectedGatts.remove(address)
      try { existing.close() } catch (_: Exception) {}
    }

    var lastError = "Connection failed"
    for (attempt in 1..3) {
      Log.i(TAG, "Attempting GATT connection to $address (attempt $attempt/3)...")
      val latch = CountDownLatch(1)
      var servicesJson = ""
      var connectError: String? = null

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
            connectedGatts.remove(address)
            try { gatt.close() } catch (_: Exception) {}
            try { notifyDisconnectedJni(address) } catch (_: Exception) {}
            if (latch.count > 0) {
              connectError = "Connection failed with status $status"
              latch.countDown()
            }
          }
        }

        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
          if (status == BluetoothGatt.GATT_SUCCESS) {
            connectedGatts[address] = gatt
            servicesJson = buildServicesJson(gatt)
            Log.i(TAG, "GATT services discovered for $address")
          } else {
            connectError = "Service discovery failed with status $status"
          }
          latch.countDown()
        }

        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
          val uuidKey = characteristic.uuid.toString().lowercase()
          val req = pendingReads.remove(uuidKey)
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
          val uuidKey = characteristic.uuid.toString().lowercase()
          val req = pendingReads.remove(uuidKey)
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
          Log.i(TAG, "onCharacteristicWrite char=${characteristic.uuid} status=$status")
          val uuidKey = characteristic.uuid.toString().lowercase()
          val req = pendingWrites.remove(uuidKey)
          if (req != null) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
              req.error = "Write failed with status $status"
            }
            req.latch.countDown()
          }
          onGattWriteCompleted()
        }

        override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
          Log.i(TAG, "onDescriptorWrite desc=${descriptor.uuid} status=$status")
          val uuidKey = "desc_${descriptor.characteristic.uuid}".lowercase()
          val req = pendingWrites.remove(uuidKey)
          if (req != null) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
              req.error = "Descriptor write failed with status $status"
            }
            req.latch.countDown()
          }
          onGattWriteCompleted()
        }

        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
          val bytes = characteristic.value ?: return
          val b64 = Base64.encodeToString(bytes, Base64.NO_WRAP)
          Log.i(TAG, "onCharacteristicChanged char=${characteristic.uuid} len=${bytes.size}")
          notifyValueJni(address, characteristic.uuid.toString(), b64)
        }

        @Suppress("DEPRECATION")
        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
          val b64 = Base64.encodeToString(value, Base64.NO_WRAP)
          Log.i(TAG, "onCharacteristicChanged (API33) char=${characteristic.uuid} len=${value.size}")
          notifyValueJni(address, characteristic.uuid.toString(), b64)
        }
      }

      mainHandler.post {
        device.connectGatt(ctx, false, callback, BluetoothDevice.TRANSPORT_LE)
      }

      val ok = latch.await(12, TimeUnit.SECONDS)
      if (ok && connectError == null && servicesJson.isNotEmpty()) {
        return servicesJson
      }
      lastError = connectError ?: if (!ok) "Connection timed out" else "No services found"
      Log.w(TAG, "Attempt $attempt failed: $lastError")
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
    val gatt = connectedGatts.remove(address) ?: return
    try {
      gatt.disconnect()
      gatt.close()
    } catch (e: Exception) {
      Log.e(TAG, "Error disconnecting GATT $address", e)
    }
  }

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

    val uuidKey = charUuid.toString().lowercase()
    val req = ReadRequest(CountDownLatch(1))
    pendingReads[uuidKey] = req

    mainHandler.post {
      if (!gatt.readCharacteristic(charac)) {
        pendingReads.remove(uuidKey)
        req.error = "readCharacteristic returned false"
        req.latch.countDown()
      }
    }

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingReads.remove(uuidKey)
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
    val uuidKey = charUuid.toString().lowercase()

    if (!withResponse) {
      enqueueGattWrite(Runnable {
        try {
          if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val resCode = gatt.writeCharacteristic(charac, bytes, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE)
            Log.i(TAG, "writeCharacteristic withoutResponse (API33) address=$address char=$charUuidStr code=$resCode")
          } else {
            charac.value = bytes
            charac.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            @Suppress("DEPRECATION")
            val ok = gatt.writeCharacteristic(charac)
            Log.i(TAG, "writeCharacteristic withoutResponse address=$address char=$charUuidStr ok=$ok")
          }
        } finally {
          onGattWriteCompleted()
        }
      })
      return "OK"
    }

    val req = WriteRequest(CountDownLatch(1))
    pendingWrites[uuidKey] = req

    enqueueGattWrite(Runnable {
      val okInitiated = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        val resCode = gatt.writeCharacteristic(charac, bytes, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT)
        resCode == BluetoothStatusCodes.SUCCESS
      } else {
        charac.value = bytes
        charac.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
        @Suppress("DEPRECATION")
        gatt.writeCharacteristic(charac)
      }

      if (!okInitiated) {
        pendingWrites.remove(uuidKey)
        req.error = "writeCharacteristic returned false"
        req.latch.countDown()
        onGattWriteCompleted()
      }
    })

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingWrites.remove(uuidKey)
    if (!ok) return "Error: Write timed out"
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

    mainHandler.post {
      val okSet = gatt.setCharacteristicNotification(charac, enable)
      Log.i(TAG, "setCharacteristicNotification address=$address char=$charUuidStr enable=$enable okSet=$okSet")
    }

    val descriptor = charac.getDescriptor(CCCD_UUID) ?: return "OK"
    val isIndicate = (charac.properties and BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0
    val valBytes = if (enable) {
      if (isIndicate) BluetoothGattDescriptor.ENABLE_INDICATION_VALUE else BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
    } else {
      BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
    }

    val descKey = "desc_${charUuid}".lowercase()
    val req = WriteRequest(CountDownLatch(1))
    pendingWrites[descKey] = req

    enqueueGattWrite(Runnable {
      try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
          descriptor.value = valBytes
          val resCode = gatt.writeDescriptor(descriptor, valBytes)
          Log.i(TAG, "subscribeCharacteristic (API33) address=$address char=$charUuidStr enable=$enable isIndicate=$isIndicate resCode=$resCode")
          if (resCode != BluetoothStatusCodes.SUCCESS) {
            pendingWrites.remove(descKey)
            req.error = "writeDescriptor returned code $resCode"
            req.latch.countDown()
            onGattWriteCompleted()
          }
        } else {
          descriptor.value = valBytes
          @Suppress("DEPRECATION")
          val ok = gatt.writeDescriptor(descriptor)
          Log.i(TAG, "subscribeCharacteristic address=$address char=$charUuidStr enable=$enable isIndicate=$isIndicate ok=$ok")
          if (!ok) {
            pendingWrites.remove(descKey)
            req.error = "writeDescriptor returned false"
            req.latch.countDown()
            onGattWriteCompleted()
          }
        }
      } catch (e: Exception) {
        Log.e(TAG, "Error in writeDescriptor for $charUuidStr", e)
        pendingWrites.remove(descKey)
        req.error = e.message
        req.latch.countDown()
        onGattWriteCompleted()
      }
    })

    val ok = req.latch.await(5, TimeUnit.SECONDS)
    pendingWrites.remove(descKey)
    if (!ok) return "Error: Subscribe timed out"
    if (req.error != null) return "Error: ${req.error}"
    return "OK"
  }

  @JvmStatic
  fun unsubscribeAll(address: String): String {
    val gatt = connectedGatts[address] ?: return "OK"
    try {
      for (service in gatt.services) {
        for (charac in service.characteristics) {
          if ((charac.properties and (BluetoothGattCharacteristic.PROPERTY_NOTIFY or BluetoothGattCharacteristic.PROPERTY_INDICATE)) != 0) {
            gatt.setCharacteristicNotification(charac, false)
            val descriptor = charac.getDescriptor(CCCD_UUID)
            if (descriptor != null) {
              descriptor.value = BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
              gatt.writeDescriptor(descriptor)
            }
          }
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
