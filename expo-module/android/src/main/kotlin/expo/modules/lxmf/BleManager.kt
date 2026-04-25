package expo.modules.lxmf

import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.Build
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

    // GATT server (peripheral role) — accepts inbound writes from remote centrals
    // and pushes outbound notifications to subscribed centrals.
    private var gattServer: BluetoothGattServer? = null
    private var serverTxChar: BluetoothGattCharacteristic? = null
    // Centrals that have enabled CCC notifications on our TX char, keyed by MAC.
    // Only these are "registered as peers" with Rust (mirrors iOS subscribedCentrals).
    private val serverSubscribers = mutableMapOf<String, BluetoothDevice>()

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
        // GATT server must be opened (and service added) before advertising,
        // otherwise centrals connect and find no service. startAdvertising()
        // is invoked from onServiceAdded once the service is registered.
        openGattServer()
        startScanning()
        mainHandler.postDelayed(txPollRunnable, TX_POLL_INTERVAL_MS)
        Log.i(TAG, "BleManager started")
    }

    fun stop() {
        stopScanning()
        stopAdvertising()
        closeGattServer()
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

    // ── GATT server (peripheral role) ────────────────────────────────────────

    private fun openGattServer() {
        val mgr = bluetoothManager ?: return
        gattServer = mgr.openGattServer(context, gattServerCallback)
        if (gattServer == null) {
            Log.e(TAG, "openGattServer returned null (missing BLUETOOTH_CONNECT?)")
            return
        }

        val service = BluetoothGattService(
            RNS_SERVICE_UUID,
            BluetoothGattService.SERVICE_TYPE_PRIMARY,
        )

        // RX — remote centrals write LXMF frames here. We forward to Rust.
        val rxChar = BluetoothGattCharacteristic(
            RNS_RX_CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_WRITE or
                BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE,
            BluetoothGattCharacteristic.PERMISSION_WRITE,
        )

        // TX — we push outbound LXMF frames to subscribed centrals via NOTIFY.
        val txChar = BluetoothGattCharacteristic(
            RNS_TX_CHAR_UUID,
            BluetoothGattCharacteristic.PROPERTY_NOTIFY or
                BluetoothGattCharacteristic.PROPERTY_READ,
            BluetoothGattCharacteristic.PERMISSION_READ,
        )
        // CCCD lets the central enable/disable notifications.
        val cccd = BluetoothGattDescriptor(
            CCCD_UUID,
            BluetoothGattDescriptor.PERMISSION_READ or
                BluetoothGattDescriptor.PERMISSION_WRITE,
        )
        txChar.addDescriptor(cccd)

        service.addCharacteristic(rxChar)
        service.addCharacteristic(txChar)
        serverTxChar = txChar

        val ok = gattServer?.addService(service) ?: false
        Log.i(TAG, "GATT server opened, addService=$ok")
    }

    private fun closeGattServer() {
        // Notify Rust that all subscribed centrals are gone.
        for ((mac, _) in serverSubscribers) {
            module.nativeBleDisconnected(macToBytes(mac))
        }
        serverSubscribers.clear()
        gattServer?.close()
        gattServer = null
        serverTxChar = null
    }

    /**
     * Push outbound bytes to a subscribed central via NOTIFY.
     * Uses the API 33+ value-bearing variant when available (more reliable on
     * Android 13+); falls back to the deprecated set-then-notify path on older
     * devices. Returns true if the notification was queued.
     */
    private fun notifyServerSubscriber(
        device: BluetoothDevice,
        txChar: BluetoothGattCharacteristic,
        data: ByteArray,
    ): Boolean {
        val server = gattServer ?: return false
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            // BluetoothStatusCodes.SUCCESS == 0
            server.notifyCharacteristicChanged(device, txChar, false, data) == 0
        } else {
            @Suppress("DEPRECATION")
            run {
                txChar.value = data
                server.notifyCharacteristicChanged(device, txChar, false)
            }
        }
    }

    private val gattServerCallback = object : BluetoothGattServerCallback() {
        override fun onServiceAdded(status: Int, service: BluetoothGattService?) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
                Log.i(TAG, "GATT service added; starting advertise")
                startAdvertising()
            } else {
                Log.e(TAG, "GATT addService failed: $status")
            }
        }

        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            val mac = device.address ?: return
            when (newState) {
                BluetoothProfile.STATE_CONNECTED -> {
                    // Just an ATT connection — peer hasn't subscribed yet, so
                    // it's not a "peer" from Rust's POV. Wait for CCC write.
                    Log.d(TAG, "GATT server: $mac connected")
                }
                BluetoothProfile.STATE_DISCONNECTED -> {
                    if (serverSubscribers.remove(mac) != null) {
                        Log.i(TAG, "GATT server: $mac disconnected (was subscribed)")
                        module.nativeBleDisconnected(macToBytes(mac))
                    }
                }
            }
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray?,
        ) {
            if (characteristic.uuid == RNS_RX_CHAR_UUID && value != null && value.isNotEmpty()) {
                // Inbound LXMF frame from a remote central. Push raw bytes to
                // Rust — HDLC deframing happens there. 4KB cap enforced in FFI.
                Log.d(TAG, "GATT server RX ${value.size}B from ${device.address}")
                module.nativeBleReceive(macToBytes(device.address), value)
            }
            if (responseNeeded) {
                gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
            }
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray?,
        ) {
            if (descriptor.uuid == CCCD_UUID
                && descriptor.characteristic?.uuid == RNS_TX_CHAR_UUID
                && value != null
            ) {
                val mac = device.address
                when {
                    value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) -> {
                        if (serverSubscribers.put(mac, device) == null) {
                            Log.i(TAG, "GATT server: $mac subscribed to TX")
                            module.nativeBleConnected(macToBytes(mac))
                        }
                    }
                    value.contentEquals(BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE) -> {
                        if (serverSubscribers.remove(mac) != null) {
                            Log.i(TAG, "GATT server: $mac unsubscribed")
                            module.nativeBleDisconnected(macToBytes(mac))
                        }
                    }
                }
            }
            if (responseNeeded) {
                gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
            }
        }

        override fun onCharacteristicReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic,
        ) {
            // TX is read+notify; some centrals do an initial read. Return empty.
            gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, ByteArray(0))
        }

        override fun onDescriptorReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            descriptor: BluetoothGattDescriptor,
        ) {
            val state = if (serverSubscribers.containsKey(device.address)) {
                BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            } else {
                BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
            }
            gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, state)
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
            // Subscribe to peer's TX char (their notify path → our inbound).
            // Convention matches iOS BLEManager.swift: RX is write-only on the
            // peripheral, TX is notify-only. Subscribing to RX is a no-op.
            val peerTxChar = service.getCharacteristic(RNS_TX_CHAR_UUID)
            if (peerTxChar != null) {
                gatt.setCharacteristicNotification(peerTxChar, true)
                val cccd = peerTxChar.getDescriptor(CCCD_UUID)
                cccd?.let {
                    @Suppress("DEPRECATION")
                    it.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                    @Suppress("DEPRECATION")
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
            if (characteristic.uuid == RNS_TX_CHAR_UUID) {
                @Suppress("DEPRECATION")
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

                // Peripheral (server) role: peer subscribed to our TX char →
                // push via NOTIFY. Mirrors iOS peripheralManager.updateValue.
                val subscriber = serverSubscribers[mac]
                val txChar = serverTxChar
                if (subscriber != null && txChar != null) {
                    val ok = notifyServerSubscriber(subscriber, txChar, data)
                    Log.d(TAG, "BLE NOTIFY ${data.size}B to $mac ok=$ok")
                    continue
                }

                // Central (client) role: write to peer's RX characteristic.
                // RX has WRITE properties; TX is notify-only (writes there
                // would be rejected by the peer). Matches iOS convention.
                val gatt = connections[mac] ?: continue
                val service = gatt.getService(RNS_SERVICE_UUID) ?: continue
                val peerRxChar = service.getCharacteristic(RNS_RX_CHAR_UUID) ?: continue
                @Suppress("DEPRECATION")
                peerRxChar.value = data
                peerRxChar.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
                @Suppress("DEPRECATION")
                val ok = gatt.writeCharacteristic(peerRxChar)
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
