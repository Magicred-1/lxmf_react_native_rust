package expo.modules.lxmf

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.exception.Exceptions
import android.content.Context
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
      nativeInit(dbPath)
    }

    AsyncFunction("start") { identityHex: String, lxmfAddressHex: String, mode: Int,
                              announceIntervalMs: Double, bleMtuHint: Int,
                              tcpHost: String?, tcpPort: Int ->
      nativeStart(identityHex, lxmfAddressHex, mode, announceIntervalMs.toLong(),
                  bleMtuHint.toShort(), tcpHost, tcpPort.toShort())
    }

    AsyncFunction("stop") {
      nativeStop()
    }

    Function("isRunning") {
      nativeIsRunning() != 0
    }

    // Messaging
    AsyncFunction("send") { destHex: String, bodyBase64: String ->
      nativeSend(destHex, bodyBase64).toDouble()
    }

    AsyncFunction("broadcast") { destsHex: List<String>, bodyBase64: String ->
      nativeBroadcast(destsHex, bodyBase64).toDouble()
    }

    // Status & State
    Function("getStatus") {
      nativeGetStatus()
    }

    Function("getBeacons") {
      nativeGetBeacons()
    }

    Function("fetchMessages") { limit: Int ->
      nativeFetchMessages(limit.toLong())
    }

    // Configuration
    Function("setLogLevel") { level: Int ->
      nativeSetLogLevel(level.toLong())
    }

    Function("abiVersion") {
      nativeAbiVersion().toInt()
    }

    // BLE Control
    Function("startBLE") {
      // Native BLE manager will be started
    }

    Function("stopBLE") {
      // Native BLE manager will be stopped
    }
  }

  // Native JNI method declarations
  private external fun nativeInit(dbPath: String?): Boolean
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
  private external fun nativeIsRunning(): Int
  private external fun nativeSend(destHex: String, bodyBase64: String): Long
  private external fun nativeBroadcast(destsHex: List<String>, bodyBase64: String): Long
  private external fun nativeGetStatus(): String?
  private external fun nativeGetBeacons(): String?
  private external fun nativeFetchMessages(limit: Long): String?
  private external fun nativeSetLogLevel(level: Long): Boolean
  private external fun nativeAbiVersion(): Long

  companion object {
    init {
      try {
        System.loadLibrary("lxmf_rn")
      } catch (e: UnsatisfiedLinkError) {
        Log.e("LxmfModule", "Failed to load liblxmf_rn: ${e.message}")
      }
    }
  }
}
