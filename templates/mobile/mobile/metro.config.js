import path from 'node:path';
import metroConfig from 'expo/metro-config.js';

import { assertQaBuildFlagFromShell } from './scripts/qa-build-flag.mjs';

const { getDefaultConfig } = metroConfig;

assertQaBuildFlagFromShell(import.meta.dirname);
const repositoryRoot = path.resolve(import.meta.dirname, '..');
const config = getDefaultConfig(import.meta.dirname);
{% if context.pwa and not context.web %}config.resolver.assetExts.push('wasm');
{% endif %}const existingBlockList = config.resolver.blockList;
const ignoredSiblings = new RegExp(
  `^${repositoryRoot.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/(?!mobile/)[^/]+/`,
);

config.watchFolders.push(repositoryRoot);
config.resolver.blockList = [
  ...(Array.isArray(existingBlockList) ? existingBlockList : [existingBlockList]),
  ignoredSiblings,
].filter(Boolean);
{% if context.auth_enabled and not context.auth_oidc %}
// Production builds swap the local Keycloak adapter for the provider SDK, so
// the bundle carries neither. QA builds keep signing in against the local realm.
const productionSignIn =
  process.env.NODE_ENV === 'production' && process.env.BAUKIT_QA_BUILD !== '1';
const localSignInAdapter = path.join(import.meta.dirname, 'src', 'sign-in', 'local.ts');
const productionSignInAdapter = path.join(import.meta.dirname, 'src', 'sign-in', 'production.ts');
const upstreamResolveRequest = config.resolver.resolveRequest;
config.resolver.resolveRequest = (context, moduleName, platform) => {
  const resolved = (upstreamResolveRequest ?? context.resolveRequest)(context, moduleName, platform);
  return productionSignIn && resolved.type === 'sourceFile' && resolved.filePath === localSignInAdapter
    ? { type: 'sourceFile', filePath: productionSignInAdapter }
    : resolved;
};
{% endif %}
export default config;
