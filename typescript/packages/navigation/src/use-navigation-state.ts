import { useEffect, useReducer } from 'react';
import {
  navigationReducer,
  resolveActiveNavigation,
  type ActiveNavigation,
  type NavigationItem,
} from './index.js';

export interface CollapseProps {
  readonly collapsed?: boolean;
  readonly defaultCollapsed?: boolean;
  readonly onCollapsedChange?: (collapsed: boolean) => void;
}

export interface NavigationStateHandle<Icon> {
  readonly active: ActiveNavigation<Icon>;
  readonly collapsed: boolean;
  readonly openIds: readonly string[];
  readonly setCollapsed: (value: boolean) => void;
  readonly toggleGroup: (id: string) => void;
}

export function useNavigationState<Icon>(
  items: readonly NavigationItem<Icon>[],
  pathname: string,
  props: CollapseProps,
): NavigationStateHandle<Icon> {
  const active = resolveActiveNavigation(items, pathname);
  const activeItem = active.item;
  const activeId =
    activeItem !== null && (activeItem.children?.length ?? 0) > 0 ? activeItem.id : null;
  const [state, dispatch] = useReducer(navigationReducer, {
    collapsed: props.defaultCollapsed ?? false,
    openIds: activeId === null ? [] : [activeId],
  });
  useEffect(() => {
    dispatch({ type: 'enter-section', id: activeId });
  }, [activeId]);
  const collapsed = props.collapsed ?? state.collapsed;
  function setCollapsed(value: boolean): void {
    dispatch({ type: 'set-collapsed', collapsed: value });
    props.onCollapsedChange?.(value);
  }
  function toggleGroup(id: string): void {
    if (collapsed) setCollapsed(false);
    dispatch({ type: 'toggle-group', id, collapsed });
  }
  return { active, collapsed, openIds: state.openIds, setCollapsed, toggleGroup };
}
