import { Stack } from 'expo-router';

const screenOptions = { headerShown: false };

export default function AuthLayout() {
  return <Stack screenOptions={screenOptions} />;
}
