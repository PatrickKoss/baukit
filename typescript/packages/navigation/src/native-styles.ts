import { StyleSheet } from 'react-native';
import { NAVIGATION_BAR_DIMENSIONS, NAVIGATION_DIMENSIONS } from './index.js';

export const styles = StyleSheet.create({
  labelGroup: { minWidth: 0, flexShrink: 1 },
  menuProfile: { padding: 8 },
  target: {
    padding: NAVIGATION_BAR_DIMENSIONS.targetPadding,
  },
  railTarget: { flexDirection: 'row', alignItems: 'center' },
  collapsedTarget: { justifyContent: 'center' },
  barTarget: { flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1 },
  bar: {
    flexDirection: 'column',
    minHeight: NAVIGATION_DIMENSIONS.nativeBar,
    borderTopWidth: NAVIGATION_BAR_DIMENSIONS.border,
    width: '100%',
  },
  rail: { borderRightWidth: 1, height: '100%' },
  slots: { flexShrink: 0 },
  barSlots: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  slot: { minWidth: 0, flexShrink: 1 },
  barBrand: { flexShrink: 0 },
  railItems: { flex: 1 },
  barItems: { width: '100%', flexGrow: 0, flexShrink: 0 },
  barContent: { flexDirection: 'row', flexGrow: 1, minWidth: '100%' },
  barSection: { flex: 1, minWidth: NAVIGATION_DIMENSIONS.target },
  children: { marginLeft: 20, paddingLeft: 8, borderLeftWidth: 1 },
  profile: { position: 'relative', marginTop: 'auto', paddingTop: 8, borderTopWidth: 1 },
  avatar: {
    width: NAVIGATION_DIMENSIONS.avatar,
    height: NAVIGATION_DIMENSIONS.avatar,
    borderRadius: NAVIGATION_DIMENSIONS.avatar / 2,
    alignItems: 'center',
    justifyContent: 'center',
  },
  modal: { flex: 1, justifyContent: 'flex-end' },
  menu: { borderWidth: 1, padding: 8, maxHeight: '80%' },
  menuItem: {
    justifyContent: 'center',
    padding: 8,
  },
  picker: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    borderWidth: 1,
    width: '100%',
  },
});
