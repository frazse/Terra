package dev.frazse.terra.desktop

import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.File
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

object LinuxBleManager {
    private var process: Process? = null
    private var writer: BufferedWriter? = null
    private var reader: BufferedReader? = null
    private val pendingRequests = ConcurrentHashMap<Long, (JSONObject?) -> Unit>()
    private val nextId = AtomicLong(1)
    private var isScanning = false

    var onJsEval: ((String) -> Unit)? = null

    fun init() {
        try {
            var rootDir: File? = File(System.getProperty("user.dir"))
            while (rootDir != null && !File(rootDir, "target/release/terra").exists()) {
                rootDir = rootDir.parentFile
            }
            val projRoot = rootDir ?: File(System.getProperty("user.dir"))
            val terraBin = File(projRoot, "target/release/terra")

            if (!terraBin.exists()) {
                println("LinuxBleManager: terra binary not found at ${terraBin.absolutePath}")
                return
            }

            println("LinuxBleManager: Launching native Rust BLE broker: ${terraBin.absolutePath} --rpc")
            val pb = ProcessBuilder(terraBin.absolutePath, "--rpc")
            val proc = pb.start()
            process = proc

            writer = BufferedWriter(OutputStreamWriter(proc.outputStream))
            reader = BufferedReader(InputStreamReader(proc.inputStream))

            Thread {
                try {
                    var line: String?
                    while (reader?.readLine().also { line = it } != null) {
                        val currentLine = line?.trim() ?: break
                        if (currentLine.startsWith("{") && currentLine.endsWith("}")) {
                            try {
                                val json = JSONObject(currentLine)
                                val evt = json.optString("event")
                                if (evt == "js_eval") {
                                    val code = json.optString("code")
                                    if (code.isNotBlank()) {
                                        onJsEval?.invoke(code)
                                    }
                                } else {
                                    val id = json.optLong("id", -1)
                                    if (id != -1L) {
                                        val callback = pendingRequests.remove(id)
                                        callback?.invoke(json)
                                    }
                                }
                            } catch (e: Exception) {
                                println("LinuxBleManager: Error parsing RPC output: ${e.message}")
                            }
                        }
                    }
                } catch (e: Exception) {
                    println("LinuxBleManager: RPC reader thread exited: ${e.message}")
                }
            }.start()

            println("LinuxBleManager: Connected to Rust BLE RPC process successfully.")
        } catch (e: Exception) {
            println("LinuxBleManager: Error initializing RPC BLE process: ${e.message}")
        }
    }

    private fun sendRpc(cmd: String, args: JSONObject = JSONObject()): String {
        val w = writer ?: return "[]"
        val id = nextId.getAndIncrement()
        val req = JSONObject()
        req.put("id", id)
        req.put("cmd", cmd)
        req.put("args", args)

        var resultStr = "[]"
        val latch = java.util.concurrent.CountDownLatch(1)

        pendingRequests[id] = { responseJson ->
            if (responseJson != null) {
                val result = responseJson.opt("result")
                resultStr = if (result != null) result.toString() else "[]"
            }
            latch.countDown()
        }

        try {
            w.write(req.toString() + "\n")
            w.flush()
            latch.await(3, java.util.concurrent.TimeUnit.SECONDS)
        } catch (e: Exception) {
            println("LinuxBleManager: RPC send error: ${e.message}")
        } finally {
            pendingRequests.remove(id)
        }

        return resultStr
    }

    fun sendRpcPublic(cmd: String, args: JSONObject = JSONObject()): String = sendRpc(cmd, args)

    fun startScan(): Boolean {
        if (!isScanning) {
            println("LinuxBleManager: Starting BLE scan via Rust RPC...")
            sendRpc("ble_start_scan")
            isScanning = true
        }
        return true
    }

    fun stopScan() {
        if (isScanning) {
            println("LinuxBleManager: Stopping BLE scan via Rust RPC...")
            sendRpc("ble_stop_scan")
            isScanning = false
        }
    }

    fun getDiscoveredDevicesJson(): String {
        val res = sendRpc("chooser_candidates")
        println("LinuxBleManager: chooser_candidates returned ($res)")
        return res
    }

    fun connect(address: String): String {
        val params = JSONObject()
        params.put("id", address)
        return sendRpc("ble_connect", params)
    }

    fun readCharacteristic(address: String, serviceUuid: String, charUuid: String): String {
        val params = JSONObject()
        params.put("id", address)
        params.put("service", serviceUuid)
        params.put("characteristic", charUuid)
        return sendRpc("ble_read", params)
    }

    fun writeCharacteristic(address: String, serviceUuid: String, charUuid: String, b64Value: String, withResponse: Boolean): String {
        val params = JSONObject()
        params.put("id", address)
        params.put("service", serviceUuid)
        params.put("characteristic", charUuid)
        params.put("value", b64Value)
        params.put("withResponse", withResponse)
        return sendRpc("ble_write", params)
    }

    fun subscribeCharacteristic(address: String, serviceUuid: String, charUuid: String, enable: Boolean): String {
        val params = JSONObject()
        params.put("id", address)
        params.put("service", serviceUuid)
        params.put("characteristic", charUuid)
        return if (enable) sendRpc("ble_subscribe", params) else sendRpc("ble_unsubscribe", params)
    }

    fun disconnect(address: String) {
        val params = JSONObject()
        params.put("id", address)
        sendRpc("ble_disconnect", params)
    }
}
