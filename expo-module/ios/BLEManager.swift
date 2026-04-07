import Foundation
import CoreBluetooth

/// Dual-role BLE manager for phone-to-phone Reticulum mesh
///
/// - Peripheral role: advertises mesh service, accepts connections from other phones
/// - Central role: scans for and connects to mesh peers
///
/// BLE data flows:
///   Inbound:  peer writes to RX characteristic → push to Rust via lxmf_on_announce / packet handler
///   Outbound: Rust node produces frames → native sends via TX characteristic notifications
class BLEManager: NSObject {
    // UUIDs matching the anon0mesh protocol
    static let meshServiceUUID = CBUUID(string: "e9e00001-bbd4-42b1-9494-0f7256199342")
    static let rxCharUUID      = CBUUID(string: "e9e00002-bbd4-42b1-9494-0f7256199342")
    static let txCharUUID      = CBUUID(string: "e9e00003-bbd4-42b1-9494-0f7256199342")
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

    /// Send data to a specific peer
    func sendToPeer(_ peerUUID: UUID, data: Data) {
        if let peripheral = connectedPeripherals[peerUUID],
           let char = txCharacteristics[peerUUID] {
            peripheral.writeValue(data, for: char, type: .withoutResponse)
        }
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
        peripheral.discoverServices([BLEManager.meshServiceUUID, BLEManager.rnodeServiceUUID])
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
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
        guard let data = characteristic.value, !data.isEmpty else { return }

        // Inbound data from a mesh peer — push into Rust node
        // The Rust layer will handle deframing, dedup, and routing
        // For now, forward raw bytes to the FFI
        // TODO: Route through lxmf_push_inbound when legacy API is wired
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
               let data = request.value {
                // Inbound write from a central peer
                // TODO: Push to Rust node
            }
            peripheral.respond(to: request, withResult: .success)
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                           didSubscribeTo characteristic: CBCharacteristic) {
        if !subscribedCentrals.contains(where: { $0.identifier == central.identifier }) {
            subscribedCentrals.append(central)
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                           didUnsubscribeFrom characteristic: CBCharacteristic) {
        subscribedCentrals.removeAll { $0.identifier == central.identifier }
    }

    // Background restoration
    func peripheralManager(_ peripheral: CBPeripheralManager, willRestoreState dict: [String: Any]) {
        // Re-setup services on restoration
        if isRunning {
            setupPeripheral()
        }
    }
}
