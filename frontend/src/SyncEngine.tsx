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
  createTicket: (type: TicketType, payload: any, status?: TicketStatus) => Promise<string>;
  updateTicket: (id: string, payload: any) => Promise<void>;
  updateTicketStatus: (id: string, status: TicketStatus) => Promise<void>;
  deleteTicket: (id: string) => Promise<void>;
  syncNow: () => Promise<void>;
  proposedActions: any[];
  acceptProposals: () => Promise<void>;
  rejectProposals: () => Promise<void>;
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

export const SyncContext = createContext<SyncContextType | undefined>(undefined);

export const SyncProvider = ({ children }: { children: ReactNode; userId?: number }) => {
  const [tickets, setTickets] = useState<TicketModel[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [connectionPhase, setConnectionPhase] = useState('idle');
  const [syncSettings, setSyncSettings] = useState<SyncSettings | null>(null);
  const [syncSession, setSyncSession] = useState<SyncSessionInfo | null>(null);
  const [proposedActions, setProposedActions] = useState<any[]>([]);

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

    // Initial load of proposals
    void fetchAgentProposals();

    return () => {
      window.removeEventListener(SYNC_CONFIG_UPDATED_EVENT, handleSyncConfigUpdated);
      removeStatusListener?.();
      removeTicketsListener?.();
      removeProposalsSyncListener?.();
    };
  }, [loadSyncConfig, loadSyncStatus, refreshTickets]);

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


  return (
    <SyncContext.Provider
      value={{
        tickets,
        isConnected,
        hasRemoteSession,
        connectionPhase,
        createTicket,
        updateTicket,
        updateTicketStatus,
        deleteTicket,
        syncNow,
        proposedActions,
        acceptProposals,
        rejectProposals,
      }}
    >
      {children}
    </SyncContext.Provider>
  );
};

