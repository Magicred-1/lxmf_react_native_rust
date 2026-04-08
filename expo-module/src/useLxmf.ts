import { useEffect, useRef, useState, useCallback } from 'react';
import { isLxmfNativeAvailable, LxmfModule, LxmfModuleNative } from './LxmfModule';

export interface LxmfNodeStatus {
  running: boolean;
  mode: number;
  identityHex: string;
  addressHex: string;
  lifecycle: number;
  epoch: number;
  pendingOutbound: number;
  outboundSent: number;
  inboundAccepted: number;
  announcesReceived: number;
  lxmfMessagesReceived: number;
}

export interface Beacon {
  destHash: string;
  state: string;
  lastAnnounce: number;
  reconnectAttempts: number;
}

export interface LxmfEvent {
  type: 'statusChanged' | 'packetReceived' | 'txReceived' | 'beaconDiscovered' | 'messageReceived' | 'announceReceived' | 'log' | 'error';
  [key: string]: any;
}

/** Node transport mode */
export enum LxmfNodeMode {
  /** BLE-only mesh (default) */
  BleOnly = 0,
  /** Connect via FFI's internal TCP (non-standard framing) */
  TcpClient = 1,
  /** Listen via FFI's internal TCP (non-standard framing) */
  TcpServer = 2,
  /** Connect to standard Reticulum daemon (rnsd) via HDLC-framed TCP */
  Reticulum = 3,
}

export interface UseLxmfOptions {
  autoStart?: boolean;
  identityHex?: string;
  lxmfAddressHex?: string;
  dbPath?: string;
  logLevel?: number;
  /** Transport mode — BLE, TCP client, or TCP server. Default: BleOnly */
  mode?: LxmfNodeMode;
  /** TCP host to connect to (client) or bind on (server). Required when mode is TCP. */
  tcpHost?: string;
  /** TCP port. Required when mode is TCP. */
  tcpPort?: number;
  /** Announce interval in ms. Default: 5000 */
  announceIntervalMs?: number;
  /** BLE MTU hint. Default: 255 */
  bleMtuHint?: number;
}

export function useLxmf(options: UseLxmfOptions = {}) {
  const [status, setStatus] = useState<LxmfNodeStatus | null>(null);
  const [beacons, setBeacons] = useState<Beacon[]>([]);
  const [events, setEvents] = useState<LxmfEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const eventBufferRef = useRef<LxmfEvent[]>([]);

  const pushEvent = useCallback((type: LxmfEvent['type'], event: Record<string, any>) => {
    const normalized = { ...event, type } as LxmfEvent;
    eventBufferRef.current.push(normalized);
    return normalized;
  }, []);

  // Initialize the module
  useEffect(() => {
    if (!isLxmfNativeAvailable) {
      setError(
        "Cannot find native module 'LxmfModule'. Run this app in an Expo development build (not Expo Go)."
      );
      return;
    }

    const init = () => {
      try {
        const dbPath = options.dbPath || null;
        const success = LxmfModule.init(dbPath);
        if (!success) {
          setError('Failed to initialize LXMF module');
          return;
        }
        // Sync running state and status on mount (node may already be running)
        const alreadyRunning = LxmfModule.isRunning();
        setRunning(alreadyRunning);
        if (alreadyRunning) {
          try {
            const statusJson = LxmfModule.getStatus();
            if (statusJson) setStatus(JSON.parse(statusJson) as LxmfNodeStatus);
          } catch {}
        }
      } catch (e: any) {
        setError(e.message);
      }
    };

    init();
  }, [options.dbPath]);

  // Event listeners
  // In Expo SDK 50+, NativeModule IS the EventEmitter (C++ implementation).
  // Call addListener() directly on the native module — NativeEventEmitter does NOT work.
  useEffect(() => {
    if (!isLxmfNativeAvailable || !LxmfModuleNative) {
      return;
    }

    // Cast to any: NativeModule<Record<never,never>> makes addListener's event names `never`,
    // but at runtime the Expo C++ EventEmitter implements addListener for all declared events.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mod = LxmfModuleNative as any;

    const subscriptions = [
      mod.addListener('onStatusChanged', (event: Record<string, any>) => {
        pushEvent('statusChanged', event);
        if (typeof event.running === 'boolean') {
          setRunning(event.running);
        }
        // Fetch complete status (event only has running+lifecycle, not identity/mode)
        try {
          const statusJson = LxmfModule.getStatus();
          if (statusJson) setStatus(JSON.parse(statusJson) as LxmfNodeStatus);
        } catch {}
      }),
      mod.addListener('onPacketReceived', (event: Record<string, any>) => {
        pushEvent('packetReceived', event);
      }),
      mod.addListener('onTxReceived', (event: Record<string, any>) => {
        pushEvent('txReceived', event);
      }),
      mod.addListener('onBeaconDiscovered', (event: Record<string, any>) => {
        pushEvent('beaconDiscovered', event);
      }),
      mod.addListener('onMessageReceived', (event: Record<string, any>) => {
        pushEvent('messageReceived', event);
      }),
      mod.addListener('onAnnounceReceived', (event: Record<string, any>) => {
        pushEvent('announceReceived', event);
      }),
      mod.addListener('onLog', (event: Record<string, any>) => {
        pushEvent('log', event);
        if (options.logLevel && options.logLevel >= (event.level as number)) {
          console.log(`[LXMF] ${String(event.message)}`);
        }
      }),
      mod.addListener('onError', (event: Record<string, any>) => {
        pushEvent('error', event);
        setError(`${String(event.code)}: ${String(event.message)}`);
      }),
    ];

    return () => {
      subscriptions.forEach((sub: { remove: () => void }) => sub.remove());
    };
  }, [options.logLevel, pushEvent]);

  // Poll events periodically
  useEffect(() => {
    const interval = setInterval(() => {
      if (eventBufferRef.current.length > 0) {
        setEvents([...eventBufferRef.current]);
        eventBufferRef.current = [];
      }
    }, 100);

    return () => clearInterval(interval);
  }, []);

  // Start/stop the node
  const start = useCallback(
    async (overrides?: {
      identityHex?: string;
      lxmfAddressHex?: string;
      mode?: LxmfNodeMode;
      tcpHost?: string;
      tcpPort?: number;
    }) => {
      try {
        if (!isLxmfNativeAvailable) {
          setError(
            "Cannot find native module 'LxmfModule'. Run this app in an Expo development build (not Expo Go)."
          );
          return false;
        }

        const resolvedIdentityHex = overrides?.identityHex ?? options.identityHex;
        const resolvedLxmfAddressHex = overrides?.lxmfAddressHex ?? options.lxmfAddressHex;
        if (!resolvedIdentityHex || !resolvedLxmfAddressHex) {
          setError('Missing identity or LXMF address. Pass them to start() or UseLxmfOptions.');
          return false;
        }

        const mode = overrides?.mode ?? options.mode ?? LxmfNodeMode.BleOnly;
        const tcpHost = overrides?.tcpHost ?? options.tcpHost ?? null;
        const tcpPort = overrides?.tcpPort ?? options.tcpPort ?? 0;
        const announceMs = options.announceIntervalMs ?? 5000;
        const bleMtu = options.bleMtuHint ?? 255;

        if (mode !== LxmfNodeMode.BleOnly && !tcpHost) {
          setError(`Mode ${mode} requires a tcpHost.`);
          return false;
        }

        await LxmfModule.start(
          resolvedIdentityHex,
          resolvedLxmfAddressHex,
          mode,
          announceMs,
          bleMtu,
          tcpHost,
          tcpPort,
        );
        setRunning(true);
        setError(null);
        return true;
      } catch (e: any) {
        setError(e.message);
        return false;
      }
    },
    [options.identityHex, options.lxmfAddressHex, options.mode, options.tcpHost, options.tcpPort, options.announceIntervalMs, options.bleMtuHint]
  );

  const stop = useCallback(async () => {
    try {
      await LxmfModule.stop();
      setRunning(false);
      setStatus(null);
    } catch (e: any) {
      setError(e.message);
    }
  }, []);

  const send = useCallback(async (destHex: string, bodyBase64: string) => {
    try {
      return await LxmfModule.send(destHex, bodyBase64);
    } catch (e: any) {
      setError(e.message);
      return -1;
    }
  }, []);

  const broadcast = useCallback(async (destsHex: string[], bodyBase64: string) => {
    try {
      return await LxmfModule.broadcast(destsHex, bodyBase64);
    } catch (e: any) {
      setError(e.message);
      return -1;
    }
  }, []);

  const getStatus = useCallback(() => {
    try {
      const statusJson = LxmfModule.getStatus();
      const parsed = statusJson ? (JSON.parse(statusJson) as LxmfNodeStatus) : null;
      if (parsed) setStatus(parsed);
      return parsed;
    } catch (e: any) {
      setError(`Failed to parse status payload: ${e?.message ?? 'unknown error'}`);
      return null;
    }
  }, []);

  const getBeacons = useCallback(() => {
    try {
      const beaconsJson = LxmfModule.getBeacons();
      return beaconsJson ? JSON.parse(beaconsJson) : [];
    } catch (e: any) {
      setError(`Failed to parse beacon payload: ${e?.message ?? 'unknown error'}`);
      return [];
    }
  }, []);

  const fetchMessages = useCallback((limit: number = 50) => {
    try {
      const messagesJson = LxmfModule.fetchMessages(limit);
      return messagesJson ? JSON.parse(messagesJson) : [];
    } catch (e: any) {
      setError(`Failed to parse message payload: ${e?.message ?? 'unknown error'}`);
      return [];
    }
  }, []);

  const setLogLevel = useCallback((level: number) => {
    return LxmfModule.setLogLevel(level);
  }, []);

  const startBLE = useCallback(() => {
    LxmfModule.startBLE();
  }, []);

  const stopBLE = useCallback(() => {
    LxmfModule.stopBLE();
  }, []);

  return {
    // State
    status,
    beacons,
    events,
    error,
    isRunning: running,
    isNativeAvailable: isLxmfNativeAvailable,

    // Methods
    start,
    stop,
    send,
    broadcast,
    getStatus,
    getBeacons,
    fetchMessages,
    setLogLevel,
    startBLE,
    stopBLE,
  };
}
