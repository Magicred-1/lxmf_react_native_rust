package expo.modules.lxmf

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import android.util.Log

class LxmfModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("LxmfModule")

    // Events
    Events(
      "onPacketReceived",
      "onTxReceived",
      "onBeaconDiscovered",
      "onMessageReceived",
      "onStatusChanged",
      "onLog",
      "onError",
      "onOutgoingPacket"
    )

    // Lifecycle
    Function("init") { dbPath: String? ->
      val rc = nativeInit(dbPath)
      if (rc != 0) throw RuntimeException("nativeInit returned $rc")
      true
    }

    AsyncFunction("start") { identityHex: String, lxmfAddressHex: String, mode: Int,
                              announceIntervalMs: Double, bleMtuHint: Int,
                              tcpHost: String?, tcpPort: Int ->
      Log.d("LxmfModule", "start() mode=$mode host=$tcpHost port=$tcpPort")
      val rc = nativeStart(identityHex, lxmfAddressHex, mode, announceIntervalMs.toLong(),
                  bleMtuHint.toShort(), tcpHost, tcpPort.toShort())
      if (rc != 0) throw RuntimeException("nativeStart returned $rc")
      true
    }

    AsyncFunction("stop") {
      val rc = nativeStop()
      if (rc != 0) throw RuntimeException("nativeStop returned $rc")
      true
    }

    Function("isRunning") {
      nativeIsRunning()
    }

    // Messaging
    AsyncFunction("send") { destHex: String, bodyBase64: String ->
      nativeSend(destHex, bodyBase64).toDouble()
    }

    AsyncFunction("broadcast") { destsHex: List<String>, bodyBase64: String ->
      val destsJson = org.json.JSONArray(destsHex).toString()
      nativeBroadcast(destsJson, bodyBase64).toDouble()
    }

    // Status & State
    Function("getStatus") {
      nativeGetStatus()
    }

    Function("getBeacons") {
      nativeGetBeacons()
    }

    Function("fetchMessages") { limit: Int ->
      nativeFetchMessages(limit)
    }

    // Configuration
    Function("setLogLevel") { level: Int ->
      nativeSetLogLevel(level) == 0
    }

    Function("abiVersion") {
      nativeAbiVersion()
    }

    // BLE Control
    Function("startBLE") {
      // Native BLE manager will be started
    }

    Function("stopBLE") {
      // Native BLE manager will be stopped
    }
  }

  // Native JNI method declarations — types must match Rust JNI signatures exactly
  private external fun nativeInit(dbPath: String?): Int
  private external fun nativeStart(
    identityHex: String,
    lxmfAddressHex: String,
    mode: Int,
    announceIntervalMs: Long,
    bleMtuHint: Short,
    tcpHost: String?,
    tcpPort: Short
  ): Int

  private external fun nativeStop(): Int
  private external fun nativeIsRunning(): Boolean
  private external fun nativeSend(destHex: String, bodyBase64: String): Long
  private external fun nativeBroadcast(destsJson: String, bodyBase64: String): Long
  private external fun nativeGetStatus(): String?
  private external fun nativeGetBeacons(): String?
  private external fun nativeFetchMessages(limit: Int): String?
  private external fun nativeSetLogLevel(level: Int): Int
  private external fun nativeAbiVersion(): Int

  companion object {
    init {
      try {
        System.loadLibrary("lxmf_rn")
        Log.i("LxmfModule", "liblxmf_rn loaded successfully")
      } catch (e: UnsatisfiedLinkError) {
        Log.e("LxmfModule", "Failed to load liblxmf_rn: ${e.message}")
      }
    }
  }
}
