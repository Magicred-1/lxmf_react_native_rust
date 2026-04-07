import ExpoModulesCore
import CoreBluetooth

// C FFI declarations — linked from the Rust staticlib (liblxmf_rn.a)
@_silgen_name("lxmf_init")
func lxmf_init(_ dbPath: UnsafePointer<CChar>?) -> Int32

@_silgen_name("lxmf_start")
func lxmf_start(
    _ identityPtr: UnsafePointer<UInt8>?,
    _ lxmfAddressPtr: UnsafePointer<UInt8>?,
    _ mode: UInt32,
    _ announceIntervalMs: UInt64,
    _ bleMtuHint: UInt16,
    _ tcpHost: UnsafePointer<CChar>?,
    _ tcpPort: UInt16
) -> Int32

@_silgen_name("lxmf_stop")
func lxmf_stop() -> Int32

@_silgen_name("lxmf_is_running")
func lxmf_is_running() -> Int32

@_silgen_name("lxmf_send")
func lxmf_send(
    _ destPtr: UnsafePointer<UInt8>?,
    _ bodyPtr: UnsafePointer<UInt8>?,
    _ bodyLen: Int
) -> Int64

@_silgen_name("lxmf_broadcast")
func lxmf_broadcast(
    _ destsPtr: UnsafePointer<UInt8>?,
    _ destCount: Int,
    _ bodyPtr: UnsafePointer<UInt8>?,
    _ bodyLen: Int
) -> Int64

@_silgen_name("lxmf_poll_events")
func lxmf_poll_events(
    _ timeoutMs: UInt64,
    _ outBuf: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32

@_silgen_name("lxmf_get_status")
func lxmf_get_status(
    _ outBuf: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32

@_silgen_name("lxmf_get_beacons")
func lxmf_get_beacons(
    _ outBuf: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32

@_silgen_name("lxmf_on_announce")
func lxmf_on_announce(
    _ destHashPtr: UnsafePointer<UInt8>?,
    _ appDataPtr: UnsafePointer<UInt8>?,
    _ appDataLen: Int
) -> Int32

@_silgen_name("lxmf_set_log_level")
func lxmf_set_log_level(_ level: UInt32) -> Int32

@_silgen_name("lxmf_abi_version")
func lxmf_abi_version() -> UInt32

@_silgen_name("lxmf_hdlc_encode")
func lxmf_hdlc_encode(
    _ dataPtr: UnsafePointer<UInt8>?,
    _ dataLen: Int,
    _ outPtr: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32

@_silgen_name("lxmf_kiss_encode")
func lxmf_kiss_encode(
    _ dataPtr: UnsafePointer<UInt8>?,
    _ dataLen: Int,
    _ outPtr: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32

@_silgen_name("lxmf_fetch_messages")
func lxmf_fetch_messages(
    _ limit: UInt32,
    _ outBuf: UnsafeMutablePointer<UInt8>?,
    _ outCapacity: Int
) -> Int32


public class LxmfModule: Module {
    // Shared JSON buffer for FFI calls (64KB)
    private var jsonBuf = [UInt8](repeating: 0, count: 65536)

    // Poll timers
    private var rxPollTimer: Timer?
    private var txDrainTimer: Timer?

    // BLE manager for phone-to-phone mesh
    private lazy var bleManager = BLEManager()

    public func definition() -> ModuleDefinition {
        Name("LxmfModule")

        // --- Events emitted to JavaScript ---
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

        // --- Lifecycle ---

        Function("init") { (dbPath: String?) -> Bool in
            let result: Int32
            if let path = dbPath {
                result = path.withCString { lxmf_init($0) }
            } else {
                result = lxmf_init(nil)
            }
            return result == 0
        }

        AsyncFunction("start") { (
            identityHex: String,
            lxmfAddressHex: String,
            mode: Int,
            announceIntervalMs: Double,
            bleMtuHint: Int,
            tcpHost: String?,
            tcpPort: Int
        ) -> Bool in
            guard let identityBytes = Self.hexToBytes(identityHex),
                  identityBytes.count == 32,
                  let addressBytes = Self.hexToBytes(lxmfAddressHex),
                  addressBytes.count == 16 else {
                return false
            }

            let result = identityBytes.withUnsafeBufferPointer { idBuf in
                addressBytes.withUnsafeBufferPointer { addrBuf in
                    if let host = tcpHost {
                        return host.withCString { hostPtr in
                            lxmf_start(
                                idBuf.baseAddress, addrBuf.baseAddress,
                                UInt32(mode), UInt64(announceIntervalMs),
                                UInt16(bleMtuHint), hostPtr, UInt16(tcpPort)
                            )
                        }
                    } else {
                        return lxmf_start(
                            idBuf.baseAddress, addrBuf.baseAddress,
                            UInt32(mode), UInt64(announceIntervalMs),
                            UInt16(bleMtuHint), nil, UInt16(tcpPort)
                        )
                    }
                }
            }

            if result == 0 {
                self.startPolling()
                self.bleManager.start()
            }

            return result == 0
        }

        AsyncFunction("stop") { () -> Bool in
            self.stopPolling()
            self.bleManager.stop()
            return lxmf_stop() == 0
        }

        Function("isRunning") { () -> Bool in
            return lxmf_is_running() != 0
        }

        // --- Messaging ---

        AsyncFunction("send") { (destHex: String, bodyBase64: String) -> Double in
            guard let destBytes = Self.hexToBytes(destHex),
                  destBytes.count == 16,
                  let bodyData = Data(base64Encoded: bodyBase64) else {
                return -1
            }

            let opId = destBytes.withUnsafeBufferPointer { destBuf in
                [UInt8](bodyData).withUnsafeBufferPointer { bodyBuf in
                    lxmf_send(destBuf.baseAddress, bodyBuf.baseAddress, bodyData.count)
                }
            }
            return Double(opId)
        }

        AsyncFunction("broadcast") { (destsHex: [String], bodyBase64: String) -> Double in
            guard let bodyData = Data(base64Encoded: bodyBase64) else { return -1 }

            var flatDests = [UInt8]()
            for hex in destsHex {
                guard let bytes = Self.hexToBytes(hex), bytes.count == 16 else { return -1 }
                flatDests.append(contentsOf: bytes)
            }

            let opId = flatDests.withUnsafeBufferPointer { destBuf in
                [UInt8](bodyData).withUnsafeBufferPointer { bodyBuf in
                    lxmf_broadcast(destBuf.baseAddress, destsHex.count, bodyBuf.baseAddress, bodyData.count)
                }
            }
            return Double(opId)
        }

        // --- Status & Beacons ---

        Function("getStatus") { () -> String? in
            return self.callJsonFfi { buf, cap in lxmf_get_status(buf, cap) }
        }

        Function("getBeacons") { () -> String? in
            return self.callJsonFfi { buf, cap in lxmf_get_beacons(buf, cap) }
        }

        Function("fetchMessages") { (limit: Int) -> String? in
            return self.callJsonFfi { buf, cap in lxmf_fetch_messages(UInt32(limit), buf, cap) }
        }

        // --- Configuration ---

        Function("setLogLevel") { (level: Int) -> Bool in
            return lxmf_set_log_level(UInt32(level)) == 0
        }

        Function("abiVersion") { () -> Int in
            return Int(lxmf_abi_version())
        }

        // --- BLE interface control ---

        Function("startBLE") { () -> Void in
            self.bleManager.start()
        }

        Function("stopBLE") { () -> Void in
            self.bleManager.stop()
        }
    }

    // MARK: - Polling

    private func startPolling() {
        // RX event poll: 80ms interval
        rxPollTimer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in
            self?.drainEvents()
        }

        // TX drain for BLE outgoing: 20ms interval
        txDrainTimer = Timer.scheduledTimer(withTimeInterval: 0.02, repeats: true) { [weak self] _ in
            self?.drainOutgoing()
        }
    }

    private func stopPolling() {
        rxPollTimer?.invalidate()
        rxPollTimer = nil
        txDrainTimer?.invalidate()
        txDrainTimer = nil
    }

    private func drainEvents() {
        let len = jsonBuf.withUnsafeMutableBufferPointer { buf in
            lxmf_poll_events(0, buf.baseAddress, buf.count)
        }

        guard len > 0 else { return }

        let jsonData = Data(jsonBuf[0..<Int(len)])
        guard let events = try? JSONSerialization.jsonObject(with: jsonData) as? [[String: Any]] else { return }

        for event in events {
            guard let type_ = event["type"] as? String else { continue }

            switch type_ {
            case "statusChanged":
                sendEvent("onStatusChanged", event)
            case "packetReceived":
                sendEvent("onPacketReceived", event)
            case "txReceived":
                sendEvent("onTxReceived", event)
            case "beaconDiscovered":
                sendEvent("onBeaconDiscovered", event)
            case "messageReceived":
                sendEvent("onMessageReceived", event)
            case "log":
                sendEvent("onLog", event)
            case "error":
                sendEvent("onError", event)
            default:
                break
            }
        }
    }

    private func drainOutgoing() {
        // Future: drain outgoing packets from Rust node and write to BLE
    }

    // MARK: - Helpers

    private func callJsonFfi(_ fn_: (UnsafeMutablePointer<UInt8>?, Int) -> Int32) -> String? {
        let len = jsonBuf.withUnsafeMutableBufferPointer { buf in
            fn_(buf.baseAddress, buf.count)
        }
        guard len > 0 else { return nil }
        return String(bytes: jsonBuf[0..<Int(len)], encoding: .utf8)
    }

    static func hexToBytes(_ hex: String) -> [UInt8]? {
        let chars = Array(hex)
        guard chars.count % 2 == 0 else { return nil }
        var bytes = [UInt8]()
        bytes.reserveCapacity(chars.count / 2)
        for i in stride(from: 0, to: chars.count, by: 2) {
            guard let byte = UInt8(String(chars[i...i+1]), radix: 16) else { return nil }
            bytes.append(byte)
        }
        return bytes
    }
}
