const { getDefaultConfig } = require('expo/metro-config');
const path = require('node:path');

const projectRoot = __dirname;
const workspaceRoot = path.resolve(projectRoot, '..');
const config = getDefaultConfig(projectRoot);

// Minimize watchers
config.maxWorkers = 1;
config.watchFolders = [
  projectRoot,
  // Include local file: dependency target so Metro can resolve symlinked package sources
  path.resolve(projectRoot, '../expo-module'),
];

// Block only heavy native source trees that are not needed for JS bundling.
const blockList = [
  /node_modules\/react-native\/ReactAndroid/,
  /node_modules\/react-native\/ReactApple/,
];

config.resolver.blockList = blockList;
config.resolver.unstable_enableSymlinks = true;
config.resolver.nodeModulesPaths = [
  path.resolve(projectRoot, 'node_modules'),
  path.resolve(workspaceRoot, 'node_modules'),
];
config.resolver.extraNodeModules = {
  react: path.resolve(projectRoot, 'node_modules/react'),
  'react-native': path.resolve(projectRoot, 'node_modules/react-native'),
};

module.exports = config;
