import { useState, useEffect, useCallback, createContext, ReactNode } from 'react';
import { v4 as uuidv4 } from 'uuid';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  SYNC_CONFIG_UPDATED_EVENT,
  resolveRemoteBaseUrl,
  type SyncMode,
  type SyncSessionInfo,
  type UserSettingsShape,
} from './syncConfig';

export interface AgentWorkspaceState {
  dock: {
    focused_app: string;
    mounted_apps: string[];
  };
  filesystem_cwd: string;
  file_tree: {
    lifecycle: string;
    cwd: string;
    entries: unknown[];
  };
  editor: {
    lifecycle: string;
    cwd: string;
    buffer: {
      path: string;
      lines: string[];
    } | null;
  };
  file_search: {
    lifecycle: string;
    focused_query: string | null;
    scope_root: string;
    matches: unknown[];
  };
  jobs: {
    lifecycle: string;
    history: unknown[];
  };
}

export interface AgentFilesystemMountState {
  host_path: string | null;
  folder_picker_supported: boolean;
}

export interface AgentToolCall {
  id: string;
  type: string;
  function: {
    name: string;
    arguments: string;
  };
}

export interface AgentSessionMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content?: string | null;
  tool_calls?: AgentToolCall[] | null;
  tool_call_id?: string | null;
  name?: string | null;
}

export interface AgentSessionState {
  messages: AgentSessionMessage[];
}

export interface AgentProgressState {
  iteration: number;
  phase: string;
  session: AgentSessionState;
}

export type TicketType = 'HABIT' | 'EVENT' | 'TASK' | 'COMMUTE' | 'COUNTDOWN';
export type TicketStatus = 'idle' | 'in_focus' | 'completed' | 'expired';

export interface TicketModel {
  id: string;
  title: string;
  type: TicketType;
  status: TicketStatus;
  payload: any;
  notes?: string;
  created_at?: string;
  updated_at?: string;
}

export type SyncActionType = 'CREATE' | 'UPDATE' | 'DELETE';

interface SyncContextType {
  tickets: TicketModel[];
  isConnected: boolean;
  hasRemoteSession: boolean;
  connectionPhase: string;
  agentWorkspace: AgentWorkspaceState | null;
  agentFilesystemMount: AgentFilesystemMountState | null;
  agentSession: AgentSessionState | null;
  agentProgress: AgentProgressState | null;
  createTicket: (type: TicketType, payload: any, status?: TicketStatus) => Promise<string>;
  updateTicket: (id: string, payload: any) => Promise<void>;
  updateTicketStatus: (id: string, status: TicketStatus) => Promise<void>;
  deleteTicket: (id: string) => Promise<void>;
  syncNow: () => Promise<void>;
  proposedActions: any[];
  acceptProposals: () => Promise<void>;
  rejectProposals: () => Promise<void>;
  pickAgentFilesystemMount: () => Promise<void>;
  clearAgentFilesystemMount: () => Promise<void>;
}

interface SyncSettings extends UserSettingsShape {
  sync_mode: SyncMode;
  custom_server_url: string | null;
}

interface SyncConnectionStatus {
  connected: boolean;
  phase: string;
  message?: string | null;
  transport_owner: string;
}

interface QueueSyncActionRequest {
  action_type: SyncActionType;
  entity_id: string;
  entity_type: string;
  payload?: any;
  status?: TicketStatus;
  notes?: string;
}

const SYNC_STATUS_EVENT = 'hstack:sync-status';
const SYNC_TICKETS_CHANGED_EVENT = 'hstack:sync-tickets-changed';
const AGENT_SESSION_SYNC_EVENT = 'AGENT_SESSION_SYNC';
const AGENT_PROGRESS_EVENT = 'AGENT_PROGRESS_UPDATE';
const AGENT_FILESYSTEM_MOUNT_SYNC_EVENT = 'AGENT_FILESYSTEM_MOUNT_SYNC';

export const SyncContext = createContext<SyncContextType | undefined>(undefined);

export const SyncProvider = ({ children }: { children: ReactNode; userId?: number }) => {
  const [tickets, setTickets] = useState<TicketModel[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [connectionPhase, setConnectionPhase] = useState('idle');
  const [syncSettings, setSyncSettings] = useState<SyncSettings | null>(null);
  const [syncSession, setSyncSession] = useState<SyncSessionInfo | null>(null);
  const [proposedActions, setProposedActions] = useState<any[]>([]);
  const [agentWorkspace, setAgentWorkspace] = useState<AgentWorkspaceState | null>(null);
  const [agentFilesystemMount, setAgentFilesystemMount] = useState<AgentFilesystemMountState | null>(null);
  const [agentSession, setAgentSession] = useState<AgentSessionState | null>(null);
  const [agentProgress, setAgentProgress] = useState<AgentProgressState | null>(null);

  const refreshTickets = useCallback(async () => {
    try {
      const nextTickets = await invoke<TicketModel[]>('get_tickets');
      setTickets(nextTickets);
    } catch (error) {
      console.error('Failed to refresh tickets from Rust sync state:', error);
    }
  }, []);

  const fetchAgentProposals = useCallback(async () => {
    try {
      const actions = await invoke<any[]>('get_agent_proposals');
      setProposedActions(actions);
    } catch (error) {
      console.error('Failed to fetch agent proposals:', error);
    }
  }, []);

  const fetchAgentWorkspace = useCallback(async () => {
    try {
      const workspace = await invoke<AgentWorkspaceState>('get_agent_workspace');
      setAgentWorkspace(workspace);
    } catch (error) {
      console.error('Failed to fetch agent workspace:', error);
    }
  }, []);

  const fetchAgentFilesystemMount = useCallback(async () => {
    try {
      const mount = await invoke<AgentFilesystemMountState>('get_agent_filesystem_mount');
      setAgentFilesystemMount(mount);
    } catch (error) {
      console.error('Failed to fetch agent filesystem mount:', error);
    }
  }, []);

  const fetchAgentSession = useCallback(async () => {
    try {
      const session = await invoke<AgentSessionState>('get_agent_session');
      setAgentSession(session);
    } catch (error) {
      console.error('Failed to fetch agent session:', error);
    }
  }, []);

  const loadSyncConfig = useCallback(async () => {
    try {
      await invoke('warm_secure_store');

      const [settings, session] = await Promise.all([
        invoke<SyncSettings>('get_settings'),
        invoke<SyncSessionInfo>('get_sync_session'),
      ]);
      setSyncSettings(settings);
      setSyncSession(session);
    } catch (error) {
      console.error('Failed to load sync configuration:', error);
      setSyncSession(null);
    }
  }, []);

  const loadSyncStatus = useCallback(async () => {
    try {
      const status = await invoke<SyncConnectionStatus>('get_sync_connection_status');
      setIsConnected(status.connected);
      setConnectionPhase(status.phase);
    } catch (error) {
      console.error('Failed to load sync status:', error);
    }
  }, []);

  useEffect(() => {
    const handleSyncConfigUpdated = () => {
      void loadSyncConfig();
    };

    void loadSyncConfig();
  void refreshTickets();
    void loadSyncStatus();
    window.addEventListener(SYNC_CONFIG_UPDATED_EVENT, handleSyncConfigUpdated);

    let removeStatusListener: (() => void) | null = null;
  let removeTicketsListener: (() => void) | null = null;

    void listen<SyncConnectionStatus>(SYNC_STATUS_EVENT, (event) => {
      setIsConnected(event.payload.connected);
      setConnectionPhase(event.payload.phase);
    }).then((unlisten) => {
      removeStatusListener = unlisten;
    });

    void listen(SYNC_TICKETS_CHANGED_EVENT, () => {
      void refreshTickets();
    }).then((unlisten) => {
      removeTicketsListener = unlisten;
    });

    let removeProposalsSyncListener: (() => void) | null = null;
    void listen('AGENT_PROPOSALS_SYNC', () => {
      void fetchAgentProposals();
    }).then((unlisten) => {
      removeProposalsSyncListener = unlisten;
    });

    let removeWorkspaceSyncListener: (() => void) | null = null;
    void listen('AGENT_WORKSPACE_SYNC', () => {
      void fetchAgentWorkspace();
    }).then((unlisten) => {
      removeWorkspaceSyncListener = unlisten;
    });

    let removeFilesystemMountSyncListener: (() => void) | null = null;
    void listen(AGENT_FILESYSTEM_MOUNT_SYNC_EVENT, () => {
      void fetchAgentFilesystemMount();
    }).then((unlisten) => {
      removeFilesystemMountSyncListener = unlisten;
    });

    let removeSessionSyncListener: (() => void) | null = null;
    void listen(AGENT_SESSION_SYNC_EVENT, () => {
      void fetchAgentSession();
    }).then((unlisten) => {
      removeSessionSyncListener = unlisten;
    });

    let removeProgressListener: (() => void) | null = null;
    void listen<AgentProgressState>(AGENT_PROGRESS_EVENT, (event) => {
      setAgentProgress(event.payload);
      setAgentSession(event.payload.session);
    }).then((unlisten) => {
      removeProgressListener = unlisten;
    });

    // Initial load of proposals
    void fetchAgentProposals();
    void fetchAgentWorkspace();
    void fetchAgentFilesystemMount();
    void fetchAgentSession();

    return () => {
      window.removeEventListener(SYNC_CONFIG_UPDATED_EVENT, handleSyncConfigUpdated);
      removeStatusListener?.();
      removeTicketsListener?.();
      removeProposalsSyncListener?.();
      removeWorkspaceSyncListener?.();
      removeFilesystemMountSyncListener?.();
      removeSessionSyncListener?.();
      removeProgressListener?.();
    };
  }, [fetchAgentFilesystemMount, fetchAgentProposals, fetchAgentSession, fetchAgentWorkspace, loadSyncConfig, loadSyncStatus, refreshTickets]);

  useEffect(() => {
    if (!syncSettings || !syncSession) {
      return;
    }

    const baseUrl = resolveRemoteBaseUrl(syncSettings);
    const hasRemoteSession = Boolean(baseUrl && syncSession.user_id && syncSession.token);

    void (async () => {
      try {
        if (hasRemoteSession && baseUrl) {
          await invoke('start_native_sync', { baseUrl });
        } else {
          await invoke('stop_native_sync');
          setIsConnected(false);
        }
      } catch (error) {
        console.error('Failed to update native sync runtime:', error);
      }
    })();
  }, [syncSettings, syncSession]);

  const hasRemoteSession = Boolean(
    syncSettings
      && resolveRemoteBaseUrl(syncSettings)
      && syncSession?.user_id
      && syncSession?.token
  );

  const queueAction = useCallback(async (action: QueueSyncActionRequest) => {
    const nextTickets = await invoke<TicketModel[]>('queue_sync_action', { action });
    setTickets(nextTickets);
  }, []);

  const createTicket = async (type: TicketType, payload: any, status: TicketStatus = 'idle') => {
      const entity_id = uuidv4();
      await queueAction({ action_type: 'CREATE', entity_id, entity_type: type, payload, status });
      return entity_id;
  };

  const updateTicket = async (id: string, payload: any) => {
      await queueAction({ action_type: 'UPDATE', entity_id: id, entity_type: 'TASK', payload });
  };

  const updateTicketStatus = async (id: string, status: TicketStatus) => {
      await queueAction({ action_type: 'UPDATE', entity_id: id, entity_type: 'TASK', status });
  };

  const deleteTicket = async (id: string) => {
      await queueAction({ action_type: 'DELETE', entity_id: id, entity_type: 'TASK' });
  };

  const syncNow = useCallback(async () => {
    try {
      const nextTickets = await invoke<TicketModel[]>('sync_refresh_now');
      setTickets(nextTickets);
    } catch (error) {
      console.error('Failed to refresh native sync state:', error);
    }
  }, []);

  const acceptProposals = async () => {
    try {
      await invoke('accept_agent_proposals');
      setProposedActions([]);
      await syncNow();
    } catch (error) {
      console.error('Failed to accept proposals:', error);
    }
  };

  const rejectProposals = async () => {
    try {
      await invoke('reject_agent_proposals');
      setProposedActions([]);
    } catch (error) {
      console.error('Failed to reject proposals:', error);
    }
  };

  const pickAgentFilesystemMount = useCallback(async () => {
    try {
      const mount = await invoke<AgentFilesystemMountState>('pick_agent_filesystem_mount');
      setAgentFilesystemMount(mount);
      await fetchAgentWorkspace();
    } catch (error) {
      console.error('Failed to pick agent filesystem mount:', error);
    }
  }, [fetchAgentWorkspace]);

  const clearAgentFilesystemMount = useCallback(async () => {
    try {
      await invoke('clear_agent_filesystem_mount');
      await Promise.all([fetchAgentFilesystemMount(), fetchAgentWorkspace()]);
    } catch (error) {
      console.error('Failed to clear agent filesystem mount:', error);
    }
  }, [fetchAgentFilesystemMount, fetchAgentWorkspace]);


  return (
    <SyncContext.Provider
      value={{
        tickets,
        isConnected,
        hasRemoteSession,
        connectionPhase,
        agentWorkspace,
        agentFilesystemMount,
        agentSession,
        agentProgress,
        createTicket,
        updateTicket,
        updateTicketStatus,
        deleteTicket,
        syncNow,
        proposedActions,
        acceptProposals,
        rejectProposals,
        pickAgentFilesystemMount,
        clearAgentFilesystemMount,
      }}
    >
      {children}
    </SyncContext.Provider>
  );
};

