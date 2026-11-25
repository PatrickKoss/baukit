import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import type { ProfileErasureMarkerStore } from '@baukit/data-contracts';

import { PRODUCT_NAME } from './product';

async function markerKey(subject: string): Promise<string> {
  const digest = await Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, subject);
  return `${PRODUCT_NAME}.profile-erasure-marker.v1.${digest}`;
}

/** Survives a crash between the server erase and the local wipe; the lifecycle finishes the wipe. */
export const profileErasureMarker: ProfileErasureMarkerStore = {
  mark: async (subject) => {
    await SecureStore.setItemAsync(await markerKey(subject), '1');
  },
  isMarked: async (subject) => (await SecureStore.getItemAsync(await markerKey(subject))) !== null,
  clear: async (subject) => {
    await SecureStore.deleteItemAsync(await markerKey(subject));
  },
};
