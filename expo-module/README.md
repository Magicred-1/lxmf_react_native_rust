# @lxmf/react-native

LXMF Reticulum mesh networking module for React Native + Expo.

## Install

```bash
npm install @lxmf/react-native
```

## Usage

```ts
import { useLxmf, LxmfNodeMode } from '@lxmf/react-native';

const lxmf = useLxmf({
  identityHex: 'new',
  lxmfAddressHex: 'new',
  mode: LxmfNodeMode.BleOnly,
});
```

## Package Contents

Published package includes:

- `build/` JavaScript + TypeScript declarations
- `android/` native Android module sources and JNI library artifacts
- `ios/` native iOS Swift sources
- `LxmfReactNative.podspec`
- Expo plugin files (`app.plugin.js`, `expo-module.config.json`)

## Build

```bash
npm run build
```

## Verify Package

```bash
npm run pack:check
```

## Publish

```bash
npm publish
```

## Important Native Note

The iOS podspec currently references a Rust static library outside the package directory:

- `../../rust-core/target/release/liblxmf_rn.a`

For public npm distribution, this static library must be bundled into the package (or built during pod install), otherwise iOS consumers will fail to link.
