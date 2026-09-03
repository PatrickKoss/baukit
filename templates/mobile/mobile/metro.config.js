import path from 'node:path';
import metroConfig from 'expo/metro-config';

const { getDefaultConfig } = metroConfig;

const repositoryRoot = path.resolve(__dirname, '..');
const config = getDefaultConfig(__dirname);
const existingBlockList = config.resolver.blockList;
const ignoredSiblings = new RegExp(
  `^${repositoryRoot.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/(?!mobile/)[^/]+/`,
);

config.watchFolders.push(repositoryRoot);
config.resolver.blockList = [
  ...(Array.isArray(existingBlockList) ? existingBlockList : [existingBlockList]),
  ignoredSiblings,
].filter(Boolean);

export default config;
