const { withInfoPlist, withAndroidManifest } = require('@expo/config-plugins');

function withLxmfPermissions(config) {
  // Android BLE permissions
  config = withAndroidManifest(config, (c) => {
    const manifest = c.modResults.manifest;
    const permissions = manifest['uses-permission'] || [];
    const blePermissions = [
      'android.permission.BLUETOOTH',
      'android.permission.BLUETOOTH_ADMIN',
      'android.permission.BLUETOOTH_SCAN',
      'android.permission.BLUETOOTH_CONNECT',
      'android.permission.BLUETOOTH_ADVERTISE',
      'android.permission.ACCESS_FINE_LOCATION',
      'android.permission.ACCESS_COARSE_LOCATION',
    ];
    for (const perm of blePermissions) {
      if (!permissions.some((p) => p.$['android:name'] === perm)) {
        permissions.push({ $: { 'android:name': perm } });
      }
    }
    manifest['uses-permission'] = permissions;
    return c;
  });

  // iOS BLE usage descriptions
  config = withInfoPlist(config, (c) => {
    c.modResults.NSBluetoothAlwaysUsageDescription =
      c.modResults.NSBluetoothAlwaysUsageDescription ||
      'Used for LXMF mesh networking via BLE';
    c.modResults.NSBluetoothPeripheralUsageDescription =
      c.modResults.NSBluetoothPeripheralUsageDescription ||
      'Used for LXMF mesh networking via BLE';
    return c;
  });

  return config;
}

module.exports = withLxmfPermissions;
