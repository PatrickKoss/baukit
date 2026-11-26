import type { ProfileErasureMarkerStore } from '@baukit/data-contracts';

import { PRODUCT_NAME } from './product';

const markerKey = (subject: string): string =>
  `${PRODUCT_NAME}.profile-erasure-marker.v1\u001f${subject}`;

/** Survives a crash between the server erase and the local wipe; the lifecycle finishes the wipe. */
export const profileErasureMarker: ProfileErasureMarkerStore = {
  mark: (subject) => {
    localStorage.setItem(markerKey(subject), '1');
    return Promise.resolve();
  },
  isMarked: (subject) => Promise.resolve(localStorage.getItem(markerKey(subject)) !== null),
  clear: (subject) => {
    localStorage.removeItem(markerKey(subject));
    return Promise.resolve();
  },
};
