import { useEffect, useRef, useState, useCallback } from 'react';
import { NativeEventEmitter } from 'react-native';
import { LxmfModule } from './LxmfModule';

const eventEmitter = new NativeEventEmitter(LxmfModule as any);

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

export interface UseLxmfOptions {
  autoStart?: boolean;
  identityHex?: string;
  lxmfAddressHex?: string;
  dbPath?: string;
  logLevel?: number;
}

export function useLxmf(options: UseLxmfOptions = {}) {
  const [status, setStatus] = useState<LxmfNodeStatus | null>(null);
  const [beacons, setBeacons] = useState<Beacon[]>([]);
  const [events, setEvents] = useState<LxmfEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const eventBufferRef = useRef<LxmfEvent[]>([]);

  // Initialize the module
  useEffect(() => {
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
    async (identityHex: string, lxmfAddressHex: string, mode: number = 0) => {
      try {
        const success = await LxmfModule.start(
          identityHex,
          lxmfAddressHex,
          mode,
          options.logLevel ? 2000 : 5000, // announce interval
          255, // BLE MTU
          null, // TCP host
          0 // TCP port
        );
        if (!success) {
          setError('Failed to start LXMF node');
        }
        return success;
      } catch (e: any) {
        setError(e.message);
        return false;
      }
    },
    [options.logLevel]
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
