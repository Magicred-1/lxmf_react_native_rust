import { NativeModule, requireOptionalNativeModule } from 'expo-modules-core';

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

const MISSING_NATIVE_MESSAGE =
  "Cannot find native module 'LxmfModule'. Use an Expo development build (not Expo Go) and rebuild native apps after local module changes.";

const LxmfModuleNative = requireOptionalNativeModule<NativeModuleType>('LxmfModule');

export const isLxmfNativeAvailable = !!LxmfModuleNative;

const throwMissingNative = (): never => {
  throw new Error(MISSING_NATIVE_MESSAGE);
};

const missingNativeShim: NativeModuleType = {
  init: () => throwMissingNative(),
  start: async () => throwMissingNative(),
  stop: async () => throwMissingNative(),
  isRunning: () => false,
  send: async () => throwMissingNative(),
  broadcast: async () => throwMissingNative(),
  getStatus: () => throwMissingNative(),
  getBeacons: () => throwMissingNative(),
  fetchMessages: () => throwMissingNative(),
  setLogLevel: () => throwMissingNative(),
  abiVersion: () => throwMissingNative(),
  startBLE: () => throwMissingNative(),
  stopBLE: () => throwMissingNative(),
} as NativeModuleType;

export const LxmfModule = LxmfModuleNative ?? missingNativeShim;
