package expo.modules.lxmf

import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import java.util.UUID

private const val TAG = "LxmfBle"

// UUIDs must match ble_iface.rs constants exactly
val RNS_SERVICE_UUID: UUID = UUID.fromString("5f3bafcd-2bb7-4de0-9c6f-2c5f88b6b8f2")
val RNS_RX_CHAR_UUID: UUID = UUID.fromString("3b28e4f6-5a30-4a5f-b700-68bb74d1b036")
val RNS_TX_CHAR_UUID: UUID = UUID.fromString("8b6ded1a-ea65-4a1e-a1f0-5cf69d5dc2ad")

// GATT descriptor for enabling notifications
val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

/**
 * BleManager — owns all Android BLE hardware access for the LXMF BLE interface.
 *
 * Responsibilities:
 *   - Scan for Reticulum BLE peers (service UUID filter)
 *   - Advertise our service UUID so peers can find us
 *   - Connect to discovered peers via GATT
 *   - Enable notifications on the RX characteristic
 *   - Pass received bytes to Rust via nativeBleReceive()
 *   - Poll nativeBlePollTx() and write results to TX characteristic
 *   - Notify Rust of connect/disconnect events
 *
 * Rust handles: HDLC framing, segmentation, Reticulum packet routing.
 */
class BleManager(
    private val context: Context,
    private val module: LxmfModule,
) {
    private val bluetoothManager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
    private val adapter: BluetoothAdapter? get() = bluetoothManager?.adapter
    private val mainHandler = Handler(Looper.getMainLooper())

    // Active GATT connections keyed by MAC address string
    private val connections = mutableMapOf<String, BluetoothGatt>()
    // MACs we are currently trying to connect (avoid duplicate attempts)
    private val connecting = mutableSetOf<String>()
    // Timestamp (ms) when each MAC last disconnected — enforces reconnect cooldown
    private val disconnectedAt = mutableMapOf<String, Long>()

    private var scanner: BluetoothLeScanner? = null
    private var advertiser: BluetoothLeAdvertiser? = null
    private var isScanning = false
    private var isAdvertising = false

    // TX polling — every 50 ms drain the Rust TX queue and write to peers
    private val txPollRunnable = object : Runnable {
        override fun run() {
            drainTxQueue()
            mainHandler.postDelayed(this, TX_POLL_INTERVAL_MS)
        }
    }

    companion object {
        private const val TX_POLL_INTERVAL_MS = 50L
        private const val SCAN_RESTART_DELAY_MS = 30_000L
        /** How long to wait before reconnecting to a peer that just disconnected. */
        private const val RECONNECT_COOLDOWN_MS = 15_000L
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    fun start() {
        if (adapter == null || !adapter!!.isEnabled) {
            Log.w(TAG, "Bluetooth not available or not enabled")
            return
        }
        startAdvertising()
        startScanning()
        mainHandler.postDelayed(txPollRunnable, TX_POLL_INTERVAL_MS)
        Log.i(TAG, "BleManager started")
    }

    fun stop() {
        stopScanning()
        stopAdvertising()
        mainHandler.removeCallbacks(txPollRunnable)
        connections.values.forEach { it.disconnect(); it.close() }
        connections.clear()
        connecting.clear()
        Log.i(TAG, "BleManager stopped")
    }

    fun connectedPeerCount(): Int = module.nativeBlePeerCount()

    // ── Advertising (so peers can find us) ───────────────────────────────────

    private fun startAdvertising() {
        advertiser = adapter?.bluetoothLeAdvertiser ?: return
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true)
            .build()
        val data = AdvertiseData.Builder()
            .addServiceUuid(ParcelUuid(RNS_SERVICE_UUID))
            .setIncludeDeviceName(false)
            .build()
        advertiser?.startAdvertising(settings, data, advertiseCallback)
        isAdvertising = true
        Log.d(TAG, "BLE advertising started")
    }

    private fun stopAdvertising() {
        if (isAdvertising) {
            advertiser?.stopAdvertising(advertiseCallback)
            isAdvertising = false
        }
    }

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            Log.i(TAG, "BLE advertise started")
        }
        override fun onStartFailure(errorCode: Int) {
            Log.e(TAG, "BLE advertise failed: $errorCode")
        }
    }

    // ── Scanning (find peers) ─────────────────────────────────────────────────

    private fun startScanning() {
        if (isScanning) return
        scanner = adapter?.bluetoothLeScanner ?: return
        val filter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(RNS_SERVICE_UUID))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .build()
        scanner?.startScan(listOf(filter), settings, scanCallback)
        isScanning = true
        Log.d(TAG, "BLE scan started")
    }

    private fun stopScanning() {
        if (isScanning) {
            scanner?.stopScan(scanCallback)
            isScanning = false
        }
    }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val device = result.device ?: return
            val mac = device.address ?: return
            if (mac in connections || mac in connecting) return
            val lastDisconnect = disconnectedAt[mac] ?: 0L
            if (System.currentTimeMillis() - lastDisconnect < RECONNECT_COOLDOWN_MS) return
            Log.i(TAG, "BLE: found peer $mac, connecting")
            connecting.add(mac)
            device.connectGatt(context, false, gattCallback, BluetoothDevice.TRANSPORT_LE)
        }
        override fun onScanFailed(errorCode: Int) {
            Log.e(TAG, "BLE scan failed: $errorCode")
            isScanning = false
            // Restart scan after delay
            mainHandler.postDelayed({ startScanning() }, SCAN_RESTART_DELAY_MS)
        }
    }

    // ── GATT callbacks ────────────────────────────────────────────────────────

    private val gattCallback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
            val mac = gatt.device.address
            when (newState) {
                BluetoothProfile.STATE_CONNECTED -> {
                    Log.i(TAG, "BLE GATT connected: $mac")
                    connections[mac] = gatt
                    connecting.remove(mac)
                    gatt.discoverServices()
                }
                BluetoothProfile.STATE_DISCONNECTED -> {
                    // Guard against double-fire (Android BLE can call this twice)
                    if (mac !in connections && mac !in connecting) return
                    Log.i(TAG, "BLE GATT disconnected: $mac (status=$status)")
                    connections.remove(mac)
                    connecting.remove(mac)
                    disconnectedAt[mac] = System.currentTimeMillis()
                    gatt.close()
                    // Notify Rust
                    module.nativeBleDisconnected(macToBytes(mac))
                }
            }
        }

        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
                Log.w(TAG, "Service discovery failed on ${gatt.device.address}: $status")
                gatt.disconnect()
                return
            }
            val service = gatt.getService(RNS_SERVICE_UUID)
            if (service == null) {
                Log.w(TAG, "RNS service not found on ${gatt.device.address}")
                gatt.disconnect()
                return
            }
            // Enable notifications on RX characteristic
            val rxChar = service.getCharacteristic(RNS_RX_CHAR_UUID)
            if (rxChar != null) {
                gatt.setCharacteristicNotification(rxChar, true)
                val cccd = rxChar.getDescriptor(CCCD_UUID)
                cccd?.let {
                    it.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                    gatt.writeDescriptor(it)
                }
            }
            // Notify Rust of new peer
            module.nativeBleConnected(macToBytes(gatt.device.address))
            Log.i(TAG, "BLE peer ready: ${gatt.device.address}")
        }

        override fun onCharacteristicChanged(
            gatt: BluetoothGatt,
            characteristic: BluetoothGattCharacteristic,
        ) {
            if (characteristic.uuid == RNS_RX_CHAR_UUID) {
                val data = characteristic.value ?: return
                Log.d(TAG, "BLE RX ${data.size}B from ${gatt.device.address}")
                module.nativeBleReceive(macToBytes(gatt.device.address), data)
            }
        }

        override fun onCharacteristicWrite(
            gatt: BluetoothGatt,
            characteristic: BluetoothGattCharacteristic,
            status: Int,
        ) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
                Log.w(TAG, "BLE TX write failed: $status on ${gatt.device.address}")
            }
        }
    }

    // ── TX drain — poll Rust and write to peer characteristics ───────────────

    private fun drainTxQueue() {
        while (true) {
            val json = module.nativeBlePollTx() ?: break
            try {
                val obj = org.json.JSONObject(json)
                val peerHex = obj.getString("peer")           // "aabbccddeeff"
                val dataB64 = obj.getString("data")
                val data = android.util.Base64.decode(dataB64, android.util.Base64.DEFAULT)
                val mac = hexToMacString(peerHex)             // "AA:BB:CC:DD:EE:FF"
                val gatt = connections[mac] ?: continue
                val service = gatt.getService(RNS_SERVICE_UUID) ?: continue
                val txChar = service.getCharacteristic(RNS_TX_CHAR_UUID) ?: continue
                txChar.value = data
                txChar.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
                val ok = gatt.writeCharacteristic(txChar)
                Log.d(TAG, "BLE TX ${data.size}B to $mac ok=$ok")
            } catch (e: Exception) {
                Log.e(TAG, "drainTxQueue parse error: ${e.message}")
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /** Convert "AA:BB:CC:DD:EE:FF" → ByteArray(6) */
    private fun macToBytes(mac: String): ByteArray {
        return mac.split(":").map { it.toInt(16).toByte() }.toByteArray()
    }

    /** Convert "aabbccddeeff" → "AA:BB:CC:DD:EE:FF" */
    private fun hexToMacString(hex: String): String {
        return hex.chunked(2).joinToString(":") { it.uppercase() }
    }
}
