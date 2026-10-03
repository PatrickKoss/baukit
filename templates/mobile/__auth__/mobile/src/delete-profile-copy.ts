export const englishErasureCopy = {
  title: 'Delete profile',
  description: 'Delete all your profile data and your sign-in account. This cannot be undone.',
  continue: 'Continue to confirmation',
  confirmation: 'Delete your profile and sign-in account permanently?',
  confirm: 'Delete profile and sign-in account',
  cancel: 'Cancel',
  busy: 'Deleting your profile and sign-in account...',
  erased: 'Your profile data and sign-in account have been deleted. You are signed out.',
  pending:
    'Your profile data has been deleted. Your sign-in account deletion is still finishing. You are signed out. You do not need to request deletion again.',
  serverFailure:
    'Deletion was rejected before your profile was erased. Your local data and session are still available. Try again.',
  ambiguous:
    'The connection was lost before deletion could be confirmed. Try again to check the same deletion request safely.',
  localFailure:
    'Your server profile was erased, but some data could not be removed from this device. Close the app and clear its local data. Contact support if you need help.',
  signoutFailure:
    'Your server profile was erased, but sign-out failed on this device. Close the app and clear its sign-in data.',
  retry: 'Try again',
  failed:
    'Your profile data was deleted, but sign-in account deletion needs help from support. Do not request deletion again.',
  checkStatus: 'Check deletion status',
  statusError:
    'Your profile data is deleted. We could not check whether sign-in account deletion has finished. It continues on the server.',
} as const;

export type ErasureCopy = {
  readonly [Key in keyof typeof englishErasureCopy]: string;
};

export const germanErasureCopy: ErasureCopy = {
  title: 'Profil löschen',
  description:
    'Alle Profildaten und dein Anmeldekonto werden gelöscht. Das kann nicht rückgängig gemacht werden.',
  continue: 'Weiter zur Bestätigung',
  confirmation: 'Profil und Anmeldekonto endgültig löschen?',
  confirm: 'Profil und Anmeldekonto löschen',
  cancel: 'Abbrechen',
  busy: 'Profil und Anmeldekonto werden gelöscht...',
  erased: 'Deine Profildaten und dein Anmeldekonto wurden gelöscht. Du bist abgemeldet.',
  pending:
    'Deine Profildaten wurden gelöscht. Die Löschung deines Anmeldekontos läuft noch. Du bist abgemeldet. Du musst die Löschung nicht erneut anfordern.',
  serverFailure:
    'Die Löschung wurde abgelehnt, bevor dein Profil gelöscht wurde. Deine lokalen Daten und deine Sitzung sind noch verfügbar. Versuche es erneut.',
  ambiguous:
    'Die Verbindung wurde unterbrochen, bevor die Löschung bestätigt werden konnte. Versuche es erneut, um dieselbe Löschanfrage sicher zu prüfen.',
  localFailure:
    'Dein Profil auf dem Server wurde gelöscht. Einige Daten auf diesem Gerät konnten nicht entfernt werden. Schließe die App und lösche ihre lokalen Daten. Wende dich bei Bedarf an den Support.',
  signoutFailure:
    'Dein Profil auf dem Server wurde gelöscht. Die Abmeldung auf diesem Gerät ist fehlgeschlagen. Schließe die App und lösche ihre Anmeldedaten.',
  retry: 'Erneut versuchen',
  failed:
    'Deine Profildaten wurden gelöscht. Für die Löschung deines Anmeldekontos ist Hilfe vom Support nötig. Fordere die Löschung nicht erneut an.',
  checkStatus: 'Löschstatus prüfen',
  statusError:
    'Deine Profildaten sind gelöscht. Wir konnten nicht prüfen, ob die Löschung des Anmeldekontos abgeschlossen ist. Sie läuft auf dem Server weiter.',
};
