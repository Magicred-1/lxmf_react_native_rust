const { withBuildProperties } = require('@expo/config-plugins');

module.exports = function withLxmfBuildConfig(config) {
  return withBuildProperties(config, {
    ios: {
      GCC_PREPROCESSOR_DEFINITIONS: ['$(inherited)', 'LXMF_ENABLE_BLE=1'],
      OTHER_LDFLAGS: ['-lc++'],
      HEADER_SEARCH_PATHS: ['$(SRCROOT)/../../../rust-core/target/release'],
      LIBRARY_SEARCH_PATHS: ['$(SRCROOT)/../../../rust-core/target/release'],
    },
  });
};
