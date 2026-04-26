const { getDefaultConfig } = require('expo/metro-config');
const path = require('node:path');

const projectRoot = __dirname;
const moduleRoot = path.resolve(projectRoot, '..');   // expo-module/
const workspaceRoot = path.resolve(projectRoot, '../..'); // repo root
const config = getDefaultConfig(projectRoot);

// Minimize watchers
config.maxWorkers = 1;
config.watchFolders = [
  projectRoot,
  // Include the parent expo-module so Metro resolves local file: dependency
  moduleRoot,
];

// Blacklist huge directories and duplicate react-native copies
const blockList = [
  /node_modules\/react-native\/ReactAndroid/,
  /node_modules\/react-native\/ReactApple/,
  // Prevent parent expo-module's react-native from shadowing the example's copy
  /expo-module\/node_modules\/react-native\//,
  /expo-module\/node_modules\/react\//,
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
