import { Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { LogBox } from 'react-native';
import { LxmfProvider } from '@/context/LxmfContext';

LogBox.ignoreLogs(['Unable to determine event arguments for "onModeChange"']);

export default function RootLayout() {
  return (
    <LxmfProvider>
      <Stack screenOptions={{ headerShown: false }} />
      <StatusBar style="light" />
    </LxmfProvider>
  );
}
