package dev.frazse.terra

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Lightweight foreground service (`connectedDevice`) kept alive while one or more
 * BLE cycling sensors/trainers are connected, preventing Android from throttling
 * WebView timers or killing BLE GATT connections when briefly backgrounded.
 */
class TerraBleService : Service() {
  companion object {
    private const val TAG = "TerraBleService"
    private const val CHANNEL_ID = "terra_ble_ride"
    private const val NOTIFICATION_ID = 4201
    private const val EXTRA_DEVICE_COUNT = "device_count"

    @Volatile
    private var running = false

    fun syncDeviceCount(context: Context?, count: Int) {
      val ctx = context?.applicationContext ?: return
      if (count <= 0) {
        stop(ctx)
        return
      }
      // Android 14+ (API 34+) requires BLUETOOTH_CONNECT (or BLUETOOTH_SCAN) for connectedDevice FGS.
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
        ContextCompat.checkSelfPermission(ctx, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED
      ) {
        return
      }
      val intent = Intent(ctx, TerraBleService::class.java).putExtra(EXTRA_DEVICE_COUNT, count)
      try {
        if (running) {
          ctx.startService(intent)
        } else {
          ContextCompat.startForegroundService(ctx, intent)
        }
      } catch (e: Exception) {
        // Avoid crashing if called while backgrounded on Android 12+ (ForegroundServiceStartNotAllowedException).
        Log.w(TAG, "Unable to start/update TerraBleService: ${e.message}")
      }
    }

    fun stop(context: Context?) {
      val ctx = context?.applicationContext ?: return
      if (!running) return
      running = false
      try {
        ctx.stopService(Intent(ctx, TerraBleService::class.java))
      } catch (e: Exception) {
        Log.w(TAG, "Unable to stop TerraBleService: ${e.message}")
      }
    }
  }

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    ensureChannel()
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val count = intent?.getIntExtra(EXTRA_DEVICE_COUNT, 1) ?: 1
    if (count <= 0) {
      running = false
      stopSelf()
      return START_NOT_STICKY
    }

    val notification = buildNotification(count)
    val fgsType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
    } else {
      0
    }
    try {
      ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, fgsType)
      running = true
    } catch (e: Exception) {
      Log.w(TAG, "startForeground failed: ${e.message}")
      running = false
      stopSelf()
    }
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    running = false
    super.onDestroy()
  }

  private fun ensureChannel() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val nm = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
    if (nm.getNotificationChannel(CHANNEL_ID) != null) return
    val channel = NotificationChannel(
      CHANNEL_ID,
      "Active Ride & Bluetooth",
      NotificationManager.IMPORTANCE_LOW
    ).apply {
      description = "Keeps Bluetooth trainers and sensors connected during a ride"
      setShowBadge(false)
    }
    nm.createNotificationChannel(channel)
  }

  private fun buildNotification(deviceCount: Int): Notification {
    val launchIntent = Intent(this, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val piFlags = PendingIntent.FLAG_UPDATE_CURRENT or
      (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) PendingIntent.FLAG_IMMUTABLE else 0)
    val pendingIntent = PendingIntent.getActivity(this, 0, launchIntent, piFlags)

    val label = if (deviceCount == 1) "1 Bluetooth device connected" else "$deviceCount Bluetooth devices connected"
    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle("Terra ride active")
      .setContentText(label)
      .setOngoing(true)
      .setSilent(true)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setContentIntent(pendingIntent)
      .build()
  }
}
