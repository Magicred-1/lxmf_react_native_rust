import Foundation
import CoreBluetooth

/// Dual-role BLE manager for phone-to-phone Reticulum mesh
///
/// - Peripheral role: advertises mesh service, accepts connections from other phones
/// - Central role: scans for and connects to mesh peers
///
/// BLE data flows:
///   Inbound:  peer writes to RX characteristic → lxmf_ble_receive → Rust BleInterface
///   Outbound: Rust BleInterface → lxmf_ble_poll_tx → TX characteristic notifications
class BLEManager: NSObject {
    // UUIDs must match ble_iface.rs constants exactly
    static let meshServiceUUID = CBUUID(string: "5f3bafcd-2bb7-4de0-9c6f-2c5f88b6b8f2")
    static let rxCharUUID      = CBUUID(string: "3b28e4f6-5a30-4a5f-b700-68bb74d1b036")
    static let txCharUUID      = CBUUID(string: "8b6ded1a-ea65-4a1e-a1f0-5cf69d5dc2ad")
    static let rnodeServiceUUID = CBUUID(string: "0000181b-0000-1000-8000-00805f9b34fb")

    // Central (scanner/client)
    private var centralManager: CBCentralManager!
    private var connectedPeripherals: [UUID: CBPeripheral] = [:]
    private var txCharacteristics: [UUID: CBCharacteristic] = [:]

    // Peripheral (advertiser/server)
    private var peripheralManager: CBPeripheralManager!
    private var rxCharacteristic: CBMutableCharacteristic?
    private var txCharacteristic: CBMutableCharacteristic?
    private var subscribedCentrals: [CBCentral] = []

    // Peer address mapping — iOS uses 128-bit UUIDs, Rust uses 6-byte addrs.
    // We derive a 6-byte pseudo-MAC from each UUID and maintain reverse mappings
    // so lxmf_ble_poll_tx frames can be routed to the correct peer.
    private var addrToPeripheralUUID: [Data: UUID] = [:]
    private var addrToCentral: [Data: CBCentral] = [:]

    private var isRunning = false

    override init() {
        super.init()
    }

    func start() {
        guard !isRunning else { return }
        isRunning = true

        // Use restoration identifiers for background BLE
        centralManager = CBCentralManager(
            delegate: self,
            queue: DispatchQueue(label: "lxmf.ble.central"),
            options: [CBCentralManagerOptionRestoreIdentifierKey: "lxmf-central"]
        )

        peripheralManager = CBPeripheralManager(
            delegate: self,
            queue: DispatchQueue(label: "lxmf.ble.peripheral"),
            options: [CBPeripheralManagerOptionRestoreIdentifierKey: "lxmf-peripheral"]
        )
    }

    func stop() {
        guard isRunning else { return }
        isRunning = false

        centralManager?.stopScan()
        for (_, peripheral) in connectedPeripherals {
            centralManager?.cancelPeripheralConnection(peripheral)
        }
        connectedPeripherals.removeAll()
        txCharacteristics.removeAll()
        addrToPeripheralUUID.removeAll()
        addrToCentral.removeAll()

        peripheralManager?.stopAdvertising()
        peripheralManager?.removeAllServices()
        subscribedCentrals.removeAll()
    }

    /// Send data to all connected peers via TX characteristic
    func sendToAll(_ data: Data) {
        // Send via peripheral role (to subscribed centrals)
        if let txChar = txCharacteristic {
            for central in subscribedCentrals {
                peripheralManager?.updateValue(data, for: txChar, onSubscribedCentrals: [central])
            }
        }

        // Send via central role (write to connected peripherals' RX)
        for (uuid, char) in txCharacteristics {
            if let peripheral = connectedPeripherals[uuid] {
                peripheral.writeValue(data, for: char, type: .withoutResponse)
            }
        }
    }

    /// Send data to a specific peer by CoreBluetooth UUID
    func sendToPeer(_ peerUUID: UUID, data: Data) {
        if let peripheral = connectedPeripherals[peerUUID],
           let char = txCharacteristics[peerUUID] {
            peripheral.writeValue(data, for: char, type: .withoutResponse)
        }
    }

    /// Send data to a specific peer by 6-byte pseudo-MAC address.
    /// Used by drainOutgoing() to route frames from lxmf_ble_poll_tx.
    func sendToPeerAddr(_ addr: Data, data: Data) {
        // Try peripheral role: send notification to a subscribed central
        if let central = addrToCentral[addr], let txChar = txCharacteristic {
            peripheralManager?.updateValue(data, for: txChar, onSubscribedCentrals: [central])
            return
        }

        // Try central role: write to connected peripheral's RX characteristic
        if let peripheralUUID = addrToPeripheralUUID[addr],
           let peripheral = connectedPeripherals[peripheralUUID],
           let char = txCharacteristics[peripheralUUID] {
            peripheral.writeValue(data, for: char, type: .withoutResponse)
        }
    }

    /// Derive a 6-byte pseudo-MAC from a CoreBluetooth UUID.
    /// XOR-folds the 16-byte UUID into 6 bytes for stable peer identification.
    static func uuidToAddr(_ uuid: UUID) -> Data {
        let u = uuid.uuid
        let bytes: [UInt8] = [u.0, u.1, u.2, u.3, u.4, u.5, u.6, u.7,
                              u.8, u.9, u.10, u.11, u.12, u.13, u.14, u.15]
        return Data([
            bytes[0] ^ bytes[6] ^ bytes[12],
            bytes[1] ^ bytes[7] ^ bytes[13],
            bytes[2] ^ bytes[8] ^ bytes[14],
            bytes[3] ^ bytes[9] ^ bytes[15],
            bytes[4] ^ bytes[10],
            bytes[5] ^ bytes[11],
        ])
    }

    // MARK: - Peripheral Setup

    private func setupPeripheral() {
        let rxChar = CBMutableCharacteristic(
            type: BLEManager.rxCharUUID,
            properties: [.write, .writeWithoutResponse],
            value: nil,
            permissions: [.writeable]
        )

        let txChar = CBMutableCharacteristic(
            type: BLEManager.txCharUUID,
            properties: [.notify, .read],
            value: nil,
            permissions: [.readable]
        )

        let service = CBMutableService(type: BLEManager.meshServiceUUID, primary: true)
        service.characteristics = [rxChar, txChar]

        self.rxCharacteristic = rxChar
        self.txCharacteristic = txChar

        peripheralManager.add(service)
    }

    private func startAdvertising() {
        peripheralManager.startAdvertising([
            CBAdvertisementDataServiceUUIDsKey: [BLEManager.meshServiceUUID],
            CBAdvertisementDataLocalNameKey: "lxmf-mesh"
        ])
    }

    // MARK: - Central Setup

    private func startScanning() {
        centralManager.scanForPeripherals(
            withServices: [BLEManager.meshServiceUUID, BLEManager.rnodeServiceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
    }
}

// MARK: - CBCentralManagerDelegate

extension BLEManager: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn && isRunning {
            startScanning()
        }
    }

    func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                        advertisementData: [String: Any], rssi RSSI: NSNumber) {
        guard connectedPeripherals[peripheral.identifier] == nil else { return }

        connectedPeripherals[peripheral.identifier] = peripheral
        peripheral.delegate = self
        central.connect(peripheral, options: nil)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        // Register peer with Rust
        let addr = BLEManager.uuidToAddr(peripheral.identifier)
        addrToPeripheralUUID[addr] = peripheral.identifier
        addr.withUnsafeBytes { ptr in
            _ = lxmf_ble_connected(ptr.baseAddress?.assumingMemoryBound(to: UInt8.self))
        }

        peripheral.discoverServices([BLEManager.meshServiceUUID, BLEManager.rnodeServiceUUID])
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        // Notify Rust of disconnection
        let addr = BLEManager.uuidToAddr(peripheral.identifier)
        addrToPeripheralUUID.removeValue(forKey: addr)
        addr.withUnsafeBytes { ptr in
            _ = lxmf_ble_disconnected(ptr.baseAddress?.assumingMemoryBound(to: UInt8.self))
        }

        connectedPeripherals.removeValue(forKey: peripheral.identifier)
        txCharacteristics.removeValue(forKey: peripheral.identifier)

        // Auto-reconnect
        if isRunning {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
                guard let self = self, self.isRunning else { return }
                central.connect(peripheral, options: nil)
            }
        }
    }

    // Background restoration
    func centralManager(_ central: CBCentralManager, willRestoreState dict: [String: Any]) {
        if let peripherals = dict[CBCentralManagerRestoredStatePeripheralsKey] as? [CBPeripheral] {
            for peripheral in peripherals {
                connectedPeripherals[peripheral.identifier] = peripheral
                peripheral.delegate = self
            }
        }
    }
}

// MARK: - CBPeripheralDelegate

extension BLEManager: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard let services = peripheral.services else { return }
        for service in services {
            peripheral.discoverCharacteristics(
                [BLEManager.rxCharUUID, BLEManager.txCharUUID],
                for: service
            )
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard let chars = service.characteristics else { return }
        for char in chars {
            if char.uuid == BLEManager.rxCharUUID {
                // This is the peer's RX — we write to it
                txCharacteristics[peripheral.identifier] = char
            } else if char.uuid == BLEManager.txCharUUID {
                // This is the peer's TX — subscribe for notifications
                peripheral.setNotifyValue(true, for: char)
            }
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard let value = characteristic.value, !value.isEmpty else { return }

        // Inbound data from a mesh peer — push into Rust BleInterface
        let addr = BLEManager.uuidToAddr(peripheral.identifier)
        addr.withUnsafeBytes { addrPtr in
            value.withUnsafeBytes { dataPtr in
                _ = lxmf_ble_receive(
                    addrPtr.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    dataPtr.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    value.count
                )
            }
        }
    }
}

// MARK: - CBPeripheralManagerDelegate

extension BLEManager: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        if peripheral.state == .poweredOn && isRunning {
            setupPeripheral()
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService, error: Error?) {
        if error == nil {
            startAdvertising()
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for request in requests {
            if request.characteristic.uuid == BLEManager.rxCharUUID,
               let value = request.value, !value.isEmpty {
                // Inbound write from a central peer — push into Rust BleInterface
                let addr = BLEManager.uuidToAddr(request.central.identifier)
                addr.withUnsafeBytes { addrPtr in
                    value.withUnsafeBytes { dataPtr in
                        _ = lxmf_ble_receive(
                            addrPtr.baseAddress?.assumingMemoryBound(to: UInt8.self),
                            dataPtr.baseAddress?.assumingMemoryBound(to: UInt8.self),
                            value.count
                        )
                    }
                }
            }
            peripheral.respond(to: request, withResult: .success)
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                           didSubscribeTo characteristic: CBCharacteristic) {
        if !subscribedCentrals.contains(where: { $0.identifier == central.identifier }) {
            subscribedCentrals.append(central)
            // Register central as a peer with Rust
            let addr = BLEManager.uuidToAddr(central.identifier)
            addrToCentral[addr] = central
            addr.withUnsafeBytes { ptr in
                _ = lxmf_ble_connected(ptr.baseAddress?.assumingMemoryBound(to: UInt8.self))
            }
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                           didUnsubscribeFrom characteristic: CBCharacteristic) {
        subscribedCentrals.removeAll { $0.identifier == central.identifier }
        // Notify Rust of central disconnection
        let addr = BLEManager.uuidToAddr(central.identifier)
        addrToCentral.removeValue(forKey: addr)
        addr.withUnsafeBytes { ptr in
            _ = lxmf_ble_disconnected(ptr.baseAddress?.assumingMemoryBound(to: UInt8.self))
        }
    }

    // Background restoration
    func peripheralManager(_ peripheral: CBPeripheralManager, willRestoreState dict: [String: Any]) {
        // Re-setup services on restoration
        if isRunning {
            setupPeripheral()
        }
    }
}
