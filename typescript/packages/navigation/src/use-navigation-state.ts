import { useEffect, useReducer } from 'react';
import { navigationReducer, resolveActiveNavigation, type NavigationItem } from './index.js';

export interface CollapseProps {
  readonly collapsed?: boolean;
  readonly defaultCollapsed?: boolean;
  readonly onCollapsedChange?: (collapsed: boolean) => void;
}

export function useNavigationState<Icon>(
  items: readonly NavigationItem<Icon>[],
  pathname: string,
  props: CollapseProps,
) {
  const active = resolveActiveNavigation(items, pathname);
  const activeId = active.item?.children?.length ? active.item.id : null;
  const [state, dispatch] = useReducer(navigationReducer, {
    collapsed: props.defaultCollapsed ?? false,
    openIds: activeId === null ? [] : [activeId],
  });
  useEffect(() => {
    dispatch({ type: 'enter-section', id: activeId });
  }, [activeId]);
  const collapsed = props.collapsed ?? state.collapsed;
  function setCollapsed(value: boolean) {
    dispatch({ type: 'set-collapsed', collapsed: value });
    props.onCollapsedChange?.(value);
  }
  function toggleGroup(id: string) {
    if (collapsed) setCollapsed(false);
    dispatch({ type: 'toggle-group', id, collapsed });
  }
  return { active, collapsed, openIds: state.openIds, setCollapsed, toggleGroup };
}
