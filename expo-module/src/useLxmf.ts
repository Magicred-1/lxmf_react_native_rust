import { useEffect, useRef, useState, useCallback } from 'react';
import { NativeEventEmitter } from 'react-native';
import { isLxmfNativeAvailable, LxmfModule } from './LxmfModule';

export interface LxmfNodeStatus {
  running: boolean;
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
  type: 'statusChanged' | 'packetReceived' | 'txReceived' | 'beaconDiscovered' | 'messageReceived' | 'log' | 'error';
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
  const eventBufferRef = useRef<LxmfEvent[]>([]);

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
        }
      } catch (e: any) {
        setError(e.message);
      }
    };

    init();
  }, [options.dbPath]);

  // Event listeners
  useEffect(() => {
    if (!isLxmfNativeAvailable) {
      return;
    }

    const eventEmitter = new NativeEventEmitter(LxmfModule as any);

    const subscriptions = [
      eventEmitter.addListener('onStatusChanged', (event) => {
        setStatus(event);
        eventBufferRef.current.push(event);
      }),
      eventEmitter.addListener('onPacketReceived', (event) => {
        eventBufferRef.current.push(event);
      }),
      eventEmitter.addListener('onTxReceived', (event) => {
        eventBufferRef.current.push(event);
      }),
      eventEmitter.addListener('onBeaconDiscovered', (event) => {
        eventBufferRef.current.push(event);
      }),
      eventEmitter.addListener('onMessageReceived', (event) => {
        eventBufferRef.current.push(event);
      }),
      eventEmitter.addListener('onLog', (event) => {
        if (options.logLevel && options.logLevel >= event.level) {
          console.log(`[LXMF] ${event.message}`);
        }
      }),
      eventEmitter.addListener('onError', (event) => {
        setError(`${event.code}: ${event.message}`);
        eventBufferRef.current.push(event);
      }),
    ];

    return () => {
      subscriptions.forEach(sub => sub.remove());
    };
  }, [options.logLevel]);

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
      return statusJson ? JSON.parse(statusJson) : null;
    } catch (e) {
      return null;
    }
  }, []);

  const getBeacons = useCallback(() => {
    try {
      const beaconsJson = LxmfModule.getBeacons();
      return beaconsJson ? JSON.parse(beaconsJson) : [];
    } catch (e) {
      return [];
    }
  }, []);

  const fetchMessages = useCallback((limit: number = 50) => {
    try {
      const messagesJson = LxmfModule.fetchMessages(limit);
      return messagesJson ? JSON.parse(messagesJson) : [];
    } catch (e) {
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
    isRunning: LxmfModule.isRunning(),
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
