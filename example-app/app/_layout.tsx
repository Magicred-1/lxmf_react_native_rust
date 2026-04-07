import React, { useEffect, useState } from 'react';
import { View, Text, ScrollView, StyleSheet } from 'react-native';
import { useLxmf } from '@lxmf/react-native';

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f5f5f5',
    paddingTop: 40,
  },
  content: {
    paddingHorizontal: 20,
    paddingVertical: 10,
  },
  title: {
    fontSize: 28,
    fontWeight: 'bold',
    marginBottom: 15,
    color: '#000',
  },
  status: {
    fontSize: 16,
    marginVertical: 8,
    color: '#333',
    fontFamily: 'monospace',
  },
  box: {
    marginTop: 15,
    padding: 12,
    borderRadius: 8,
    backgroundColor: '#e0f7e0',
  },
  error: {
    backgroundColor: '#ffe0e0',
    color: '#d00',
    padding: 12,
    borderRadius: 8,
    marginTop: 15,
  },
});

export default function RootLayout() {
  const { isRunning, error, start } = useLxmf();
  const [message, setMessage] = useState('Starting...');

  useEffect(() => {
    console.log('[Root] Component mounted, initializing LXMF');
    start()
      .then(() => {
        setMessage('✅ LXMF Node Ready');
        console.log('[Root] Node initialized successfully');
      })
      .catch((err: any) => {
        console.error('[Root] Init error:', err);
        setMessage(`Error: ${err.message}`);
      });
  }, []);

  return (
    <ScrollView style={styles.container}>
      <View style={styles.content}>
        <Text style={styles.title}>🚀 LXMF React Native</Text>
        
        <Text style={styles.status}>Status: {isRunning ? '✅ RUNNING' : '⏳ STARTING'}</Text>
        <Text style={styles.status}>{message}</Text>

        {error && <Text style={styles.error}>{error}</Text>}
        
        {isRunning && (
          <View style={styles.box}>
            <Text style={{ fontSize: 14, color: '#000' }}>
              ✨ Native module is connected and working!
            </Text>
          </View>
        )}

        <Text style={{ marginTop: 30, fontSize: 12, color: '#666' }}>
          This app uses Expo with native Rust/Reticulum bridge via LxmfModule (iOS/Android).
        </Text>
      </View>
    </ScrollView>
  );
}
