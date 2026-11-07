import { useId, useRef, useState, type ReactNode } from 'react';
import { Platform, Text, View } from 'react-native';
import { announce, type OverlayBackgroundProps } from '@baukit/a11y-core';
import { resolveActiveNavigation, type NavigationItem } from './index.js';
import { Menu } from './native-menu.js';
import { Decorative, FocusablePressable } from './native-primitives.js';
import { styles } from './native-styles.js';
import {
  textStyle,
  type Navigate,
  type NavigationIcon,
  type NavigationInsets,
  type NavigationTheme,
} from './native-theme.js';

export interface SectionPickerProps {
  readonly replace?: boolean;
  readonly closeLabel: string;
  readonly item: NavigationItem<NavigationIcon>;
  readonly pathname: string;
  readonly onNavigate: Navigate;
  readonly theme: NavigationTheme;
  readonly insets?: NavigationInsets;
}
export function SectionPicker({
  replace = false,
  item,
  pathname,
  onNavigate,
  theme,
  closeLabel,
  insets,
}: SectionPickerProps): ReactNode {
  const [open, setOpen] = useState(false);
  const [openedAt, setOpenedAt] = useState(pathname);
  if (openedAt !== pathname) {
    setOpenedAt(pathname);
    setOpen(false);
  }
  const triggerRef = useRef<View>(null);
  const id = useId();
  const active = resolveActiveNavigation([item], pathname).subItem ?? item.children?.[0];
  const entries = item.children ?? [];
  if (entries.length === 0) return null;
  const renderBackground = (backgroundProps: OverlayBackgroundProps) => (
    <View {...backgroundProps} role="navigation" accessibilityLabel={item.label}>
      <FocusablePressable
        theme={theme}
        unfocusedBorderColor={theme.border}
        ref={triggerRef}
        {...(Platform.OS === 'web'
          ? { 'aria-controls': id, 'aria-haspopup': 'menu' as const }
          : {})}
        accessibilityRole="button"
        accessibilityLabel={`${item.label}, ${active?.label ?? ''}`}
        accessibilityState={{ expanded: open }}
        onPress={() => {
          setOpen(true);
        }}
        style={[
          styles.picker,
          {
            backgroundColor: theme.background,
            borderColor: theme.border,
            borderRadius: theme.radius,
            padding: theme.spacing,
          },
        ]}
      >
        <View>
          <Text style={{ ...textStyle(theme), color: theme.muted }}>{item.label}</Text>
          <Text style={{ ...textStyle(theme), color: theme.text }}>{active?.label}</Text>
        </View>
        <Decorative>
          <Text style={{ ...textStyle(theme), color: theme.text }}>⌄</Text>
        </Decorative>
      </FocusablePressable>
    </View>
  );
  return (
    <Menu
      renderBackground={renderBackground}
      entries={entries}
      closeLabel={closeLabel}
      pathname={pathname}
      label={item.label}
      visible={open}
      id={id}
      triggerRef={triggerRef}
      theme={theme}
      insets={insets}
      onClose={() => {
        setOpen(false);
      }}
      onNavigate={(href) => {
        announce(entries.find((entry) => entry.href === href)?.label ?? item.label);
        if (replace) onNavigate(href, { replace: true });
        else onNavigate(href);
      }}
    />
  );
}
