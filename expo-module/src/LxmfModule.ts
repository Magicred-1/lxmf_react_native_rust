import { NativeModule, requireNativeModule } from 'expo-modules-core';

// Import the native module
const LxmfModuleNative = requireNativeModule('LxmfModule') as NativeModuleType;

export interface NativeModuleType extends NativeModule {
  // Lifecycle
  init(dbPath?: string | null): boolean;
  start(
    identityHex: string,
    lxmfAddressHex: string,
    mode: number,
    announceIntervalMs: number,
    bleMtuHint: number,
    tcpHost: string | null,
    tcpPort: number
  ): Promise<boolean>;
  stop(): Promise<boolean>;
  isRunning(): boolean;

  // Messaging
  send(destHex: string, bodyBase64: string): Promise<number>;
  broadcast(destsHex: string[], bodyBase64: string): Promise<number>;

  // Status & State
  getStatus(): string | null;
  getBeacons(): string | null;
  fetchMessages(limit: number): string | null;

  // Configuration
  setLogLevel(level: number): boolean;
  abiVersion(): number;

  // BLE Control
  startBLE(): void;
  stopBLE(): void;
}

export const LxmfModule = LxmfModuleNative;
