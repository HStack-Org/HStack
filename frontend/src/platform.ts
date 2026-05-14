import { invoke } from '@tauri-apps/api/core';

const MOBILE_USER_AGENT_PATTERN = /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini/i;

const isTouchMac = () => {
  if (typeof navigator === 'undefined') {
    return false;
  }

  return navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1;
};

export const isMobileDevice = () => {
  if (typeof navigator === 'undefined') {
    return false;
  }

  return MOBILE_USER_AGENT_PATTERN.test(navigator.userAgent) || isTouchMac();
};

export const canUseDesktopWindowControls = () => !isMobileDevice();

const getDesktopWindow = async () => {
  if (!canUseDesktopWindowControls()) {
    return null;
  }

  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow();
};

export const minimizeDesktopWindow = async () => {
  const currentWindow = await getDesktopWindow();
  if (!currentWindow) {
    return;
  }

  await currentWindow.minimize();
};

export const closeDesktopWindow = async () => {
  const currentWindow = await getDesktopWindow();
  if (!currentWindow) {
    return;
  }

  await currentWindow.close();
};

export const startDesktopWindowDrag = async () => {
  const currentWindow = await getDesktopWindow();
  if (!currentWindow) {
    return;
  }

  await currentWindow.startDragging();
};

export const minimizeAgentWorkspaceWindow = async () => {
  await invoke('minimize_agent_workspace_window');
};

export const closeAgentWorkspaceWindow = async () => {
  await invoke('close_agent_workspace_window');
};

export const startAgentWorkspaceDrag = async () => {
  await invoke('start_agent_workspace_drag');
};

export const openAgentWorkspaceWindow = async () => {
  if (!canUseDesktopWindowControls()) {
    return;
  }

  await invoke('open_agent_workspace_window');
};