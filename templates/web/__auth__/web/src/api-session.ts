let confirmedProfileId: string | undefined;

/** Stamps API requests with the confirmed profile so stale 401s cannot sign out a newer one. */
export function setApiSessionSubject(profileId: string | undefined): void {
  confirmedProfileId = profileId;
}

export function apiSessionSubject(): string | undefined {
  return confirmedProfileId;
}
