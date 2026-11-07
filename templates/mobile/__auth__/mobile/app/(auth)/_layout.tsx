import type { ReactElement } from 'react';
import { Stack } from 'expo-router';

const screenOptions = { headerShown: false };

export default function AuthLayout(): ReactElement {
  return <Stack screenOptions={screenOptions} />;
}
