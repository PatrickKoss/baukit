const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);
config.watchFolders = [
  ...config.watchFolders,
  path.resolve(__dirname, "../../fixtures"),
];

module.exports = config;
