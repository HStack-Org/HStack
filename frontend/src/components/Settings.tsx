import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Key, X } from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { WebGLGrain } from "./WebGLGrain";
import {
    REMOTE_GOOGLE_PROVIDER,
    REMOTE_OAUTH_REDIRECT_EVENT,
    authenticateRemote,
    clearRemoteSession,
    completeRemoteOAuth,
    formatRemoteUserName,
    parseRemoteOAuthRedirectUrl,
    saveRemoteSession,
    startGoogleRemoteOAuth,
    type RemoteAuthMode,
    resolveAuthBaseUrl,
} from "../syncAuth";
import { buildApiUrl, normalizeSyncBaseUrl, notifySyncConfigUpdated, resolveRemoteSyncConfig, type SyncSessionInfo } from "../syncConfig";
import { useI18n } from "../i18n";
import { LocaleSection, HostingSyncSection, ProviderModal, ProvidersSection, SavedLocationsSection, VoiceSection } from "./settings/sections";
import { OPENAI_DEFAULT_ENDPOINT, OPENAI_DEFAULT_MODEL, type GeminiModelSummary, type ProviderDraft, type SavedProvider, type UserSettings, type VoiceCapabilityResponse, type VoiceSecretStatus } from "./settings/types";

interface SettingsProps {
    isOpen: boolean;
    onClose: () => void;
}

export const Settings = ({ isOpen, onClose }: SettingsProps) => {
    const { t } = useI18n();
    const [settings, setSettings] = useState<UserSettings | null>(null);
    const [syncSession, setSyncSession] = useState<SyncSessionInfo | null>(null);
    const [isAdding, setIsAdding] = useState(false);
    const [syncAuthMode, setSyncAuthMode] = useState<RemoteAuthMode>('login');
    const [syncLoginEmail, setSyncLoginEmail] = useState('');
    const [syncFirstName, setSyncFirstName] = useState('');
    const [syncLastName, setSyncLastName] = useState('');
    const [syncEmail, setSyncEmail] = useState('');
    const [syncPassword, setSyncPassword] = useState('');
    const [syncPending, setSyncPending] = useState(false);
    const [syncError, setSyncError] = useState<string | null>(null);
    const [voiceSecretStatus, setVoiceSecretStatus] = useState<VoiceSecretStatus | null>(null);
    const [voiceCapability, setVoiceCapability] = useState<VoiceCapabilityResponse | null>(null);
    const [voiceCapabilityError, setVoiceCapabilityError] = useState<string | null>(null);
    const [voiceApiKeyDraft, setVoiceApiKeyDraft] = useState('');
    const [geminiModels, setGeminiModels] = useState<GeminiModelSummary[]>([]);
    const [isLoadingGeminiModels, setIsLoadingGeminiModels] = useState(false);
    const [geminiModelsError, setGeminiModelsError] = useState<string | null>(null);
    const [newSavedLocation, setNewSavedLocation] = useState({ label: '', address: '' });
    const [newProvider, setNewProvider] = useState<ProviderDraft>({
        name: "",
        kind: "OpenAiCompatible",
        endpoint: OPENAI_DEFAULT_ENDPOINT,
        apiKey: "",
        model_name: OPENAI_DEFAULT_MODEL
    });
    const remoteBaseUrl = settings ? resolveAuthBaseUrl(settings.sync_mode, settings.custom_server_url) : null;
    const hasRemoteMode = settings?.sync_mode === 'CloudOfficial' || settings?.sync_mode === 'CloudCustom';
    const hasRemoteSession = Boolean(syncSession?.token && syncSession.user_id);

    useEffect(() => {
        if (isOpen) {
            void loadSettings();
        }
    }, [isOpen]);

    useEffect(() => {
        if (!isOpen || !settings || !hasRemoteMode) {
            return;
        }

        const handleOAuthRedirect = async (event: Event) => {
            const urls = (event as CustomEvent<string[]>).detail || [];
            const baseUrl = resolveAuthBaseUrl(settings.sync_mode, settings.custom_server_url);
            if (!baseUrl) {
                return;
            }

            const redirect = urls
                .map((url) => parseRemoteOAuthRedirectUrl(url))
                .find((value) => value?.provider === REMOTE_GOOGLE_PROVIDER);

            if (!redirect) {
                return;
            }

            if (redirect.error) {
                setSyncError(redirect.errorDescription || redirect.error);
                setSyncPending(false);
                return;
            }

            if (!redirect.code) {
                setSyncError(t('googleAuthMissingCode'));
                setSyncPending(false);
                return;
            }

            try {
                setSyncPending(true);
                setSyncError(null);

                const authResult = await completeRemoteOAuth(baseUrl, redirect.code);
                await saveRemoteSession(authResult);

                const normalizedCustomUrl = settings.sync_mode === 'CloudCustom'
                    ? normalizeSyncBaseUrl(settings.custom_server_url)
                    : settings.custom_server_url;
                const userName = formatRemoteUserName(authResult.user);
                const updatedSettings = {
                    ...settings,
                    custom_server_url: normalizedCustomUrl,
                    sync_user_id: authResult.user.id,
                    sync_user_name: userName,
                };

                if (settings.sync_mode === 'CloudCustom' && normalizedCustomUrl !== settings.custom_server_url) {
                    await persistSettings(updatedSettings);
                } else {
                    setSettings(updatedSettings);
                }

                setSyncSession({
                    user_id: authResult.user.id,
                    user_name: userName,
                    token: authResult.token,
                });
                setSyncLoginEmail('');
                setSyncFirstName('');
                setSyncLastName('');
                setSyncEmail('');
                setSyncPassword('');
                notifySyncConfigUpdated();
            } catch (error) {
                console.error('Failed to complete sync Google OAuth:', error);
                setSyncError(error instanceof Error ? error.message : t('remoteAuthFailed'));
            } finally {
                setSyncPending(false);
            }
        };

        window.addEventListener(REMOTE_OAUTH_REDIRECT_EVENT, handleOAuthRedirect as EventListener);
        return () => {
            window.removeEventListener(REMOTE_OAUTH_REDIRECT_EVENT, handleOAuthRedirect as EventListener);
        };
    }, [hasRemoteMode, isOpen, settings, t]);

    const refreshVoiceCapability = async (loadedSettings: UserSettings, loadedSession: SyncSessionInfo) => {
        const remoteConfig = resolveRemoteSyncConfig(loadedSettings, loadedSession);

        if (!remoteConfig) {
            setVoiceCapability(null);
            setVoiceCapabilityError(null);
            return;
        }

        try {
            const response = await fetch(buildApiUrl(remoteConfig.baseUrl, '/api/voice/capability'), {
                headers: {
                    Authorization: `Bearer ${remoteConfig.token}`,
                },
            });

            if (!response.ok) {
                throw new Error(`Capability request failed: ${response.status}`);
            }

            const capability = await response.json() as VoiceCapabilityResponse;
            setVoiceCapability(capability);
            setVoiceCapabilityError(null);
        } catch (error) {
            console.error('Failed to load voice capability:', error);
            setVoiceCapability(null);
            setVoiceCapabilityError(error instanceof Error ? error.message : 'Failed to load managed voice capability.');
        }
    };

    const loadSettings = async () => {
        try {
            const [loadedSettings, loadedSession, loadedVoiceSecretStatus] = await Promise.all([
                invoke<UserSettings>("get_settings"),
                invoke<SyncSessionInfo>("get_sync_session"),
                invoke<VoiceSecretStatus>("get_voice_secret_status"),
            ]);
            setSettings(loadedSettings);
            setSyncSession(loadedSession);
            setVoiceSecretStatus(loadedVoiceSecretStatus);
            setSyncError(null);
            setSyncPassword('');
            await refreshVoiceCapability(loadedSettings, loadedSession);
        } catch (err) {
            console.error("Failed to load settings:", err);
        }
    };

    const persistSettings = async (updated: UserSettings) => {
        await invoke("save_settings", { settings: updated });
        setSettings(updated);
    };

    const handleEditProvider = (p: SavedProvider) => {
        setNewProvider({
            id: p.id,
            name: p.name,
            kind: p.kind,
            endpoint: p.endpoint,
            model_name: p.model_name,
            apiKey: "" // Explicitly blank, meaning 'keep existing' on backend
        });
        setIsAdding(true);
    };

    const handleAddNew = () => {
        setNewProvider({
            id: undefined, // Force new UUID on save
            name: "",
            kind: "OpenAiCompatible",
            endpoint: OPENAI_DEFAULT_ENDPOINT,
            apiKey: "",
            model_name: OPENAI_DEFAULT_MODEL
        });
        setGeminiModels([]);
        setGeminiModelsError(null);
        setIsAdding(true);
    };

    const updateNewProvider = (patch: Partial<ProviderDraft>) => {
        setNewProvider((current) => ({ ...current, ...patch }));
    };

    const fetchGeminiModels = async (apiKey: string) => {
        const trimmedKey = apiKey.trim();
        if (!trimmedKey) {
            setGeminiModels([]);
            setGeminiModelsError(null);
            return;
        }

        try {
            setIsLoadingGeminiModels(true);
            setGeminiModelsError(null);

            const collected = new Map<string, GeminiModelSummary>();
            let nextPageToken: string | null = null;

            do {
                const url = new URL('https://generativelanguage.googleapis.com/v1beta/models');
                url.searchParams.set('key', trimmedKey);
                url.searchParams.set('pageSize', '1000');
                if (nextPageToken) {
                    url.searchParams.set('pageToken', nextPageToken);
                }

                const response = await fetch(url.toString());
                if (!response.ok) {
                    throw new Error(`Gemini models request failed: ${response.status}`);
                }

                const payload = await response.json() as {
                    models?: Array<{
                        name?: string;
                        baseModelId?: string;
                        displayName?: string;
                        supportedGenerationMethods?: string[];
                    }>;
                    nextPageToken?: string;
                };

                for (const model of payload.models || []) {
                    const supportsGeneration = model.supportedGenerationMethods?.includes('generateContent');
                    const modelName = model.baseModelId || model.name?.replace(/^models\//, '') || '';
                    if (!supportsGeneration || !modelName.startsWith('gemini')) {
                        continue;
                    }

                    if (!collected.has(modelName)) {
                        collected.set(modelName, {
                            name: modelName,
                            label: model.displayName?.trim() || modelName,
                        });
                    }
                }

                nextPageToken = payload.nextPageToken || null;
            } while (nextPageToken);

            const sortedModels = Array.from(collected.values()).sort((left, right) => left.label.localeCompare(right.label));
            setGeminiModels(sortedModels);

            setNewProvider((current) => {
                if (current.kind !== 'Gemini') {
                    return current;
                }

                if (current.model_name && sortedModels.some((model) => model.name === current.model_name)) {
                    return current;
                }

                return {
                    ...current,
                    model_name: sortedModels[0]?.name || current.model_name || '',
                };
            });
        } catch (error) {
            console.error('Failed to load Gemini models:', error);
            setGeminiModels([]);
            setGeminiModelsError(error instanceof Error ? error.message : 'Failed to load Gemini models.');
        } finally {
            setIsLoadingGeminiModels(false);
        }
    };

    const handleProviderKindChange = (kind: SavedProvider["kind"]) => {
        setNewProvider((current) => {
            if (kind === 'Gemini') {
                return {
                    ...current,
                    kind,
                    endpoint: '',
                    model_name: current.kind === 'Gemini' ? (current.model_name || '') : '',
                };
            }

            return {
                ...current,
                kind,
                endpoint: current.endpoint?.trim() ? current.endpoint : OPENAI_DEFAULT_ENDPOINT,
                model_name:
                    current.kind === 'OpenAiCompatible' && current.model_name?.trim()
                        ? current.model_name
                        : OPENAI_DEFAULT_MODEL,
            };
        });

        if (kind !== 'Gemini') {
            setGeminiModels([]);
            setGeminiModelsError(null);
            setIsLoadingGeminiModels(false);
        }
    };

    useEffect(() => {
        if (!isAdding || newProvider.kind !== 'Gemini') {
            return;
        }

        const apiKey = newProvider.apiKey?.trim();
        if (!apiKey) {
            setGeminiModels([]);
            setGeminiModelsError(null);
            setIsLoadingGeminiModels(false);
            return;
        }

        const timer = window.setTimeout(() => {
            void fetchGeminiModels(apiKey);
        }, 300);

        return () => {
            window.clearTimeout(timer);
        };
    }, [isAdding, newProvider.kind, newProvider.apiKey]);

    const handleUpsertProvider = async () => {
        if (!settings || !newProvider.name || !newProvider.model_name) return;

        const provider: SavedProvider = {
            id: newProvider.id || crypto.randomUUID(),
            name: newProvider.name,
            kind: (newProvider.kind as any) || "OpenAiCompatible",
            endpoint: newProvider.kind === 'Gemini' ? "" : (newProvider.endpoint || ""),
            model_name: newProvider.model_name,
        };

        try {
            await invoke("upsert_provider", { 
                provider, 
                // Only send apiKey if they typed something. 
                // In Rust, api_key parameter is Option<String>.
                apiKey: newProvider.apiKey ? newProvider.apiKey : null 
            });
            await loadSettings();
            setIsAdding(false);
        } catch (err) {
            console.error("Failed to save provider:", err);
        }
    };

    const handleDeleteProvider = async (id: string) => {
        try {
            await invoke("delete_provider", { id });
            await loadSettings();
        } catch (err) {
            console.error("Failed to delete provider:", err);
        }
    };

    const setDefault = async (id: string) => {
        if (!settings) return;
        const updated = { ...settings, default_provider_id: id };
        try {
            await persistSettings(updated);
        } catch (err) {
            console.error("Failed to set default:", err);
        }
    };

    const handleSyncModeChange = async (syncMode: UserSettings["sync_mode"]) => {
        if (!settings || settings.sync_mode === syncMode) return;

        try {
            await clearRemoteSession();
            const updated = {
                ...settings,
                sync_mode: syncMode,
                sync_user_id: null,
                sync_user_name: null,
            };
            await persistSettings(updated);
            setSyncSession({ user_id: null, user_name: null, token: null });
            setSyncAuthMode('login');
            setSyncLoginEmail('');
            setSyncFirstName('');
            setSyncLastName('');
            setSyncEmail('');
            setSyncPassword('');
            setSyncError(null);
            notifySyncConfigUpdated();
        } catch (err) {
            console.error("Failed to update hosting mode:", err);
        }
    };

    const handleRemoteAuth = async () => {
        if (!settings || syncPending) return;

        try {
            setSyncPending(true);
            setSyncError(null);

            const baseUrl = resolveAuthBaseUrl(settings.sync_mode, settings.custom_server_url);

            if (!baseUrl) {
                throw new Error(
                    settings.sync_mode === 'CloudOfficial'
                        ? t('officialCloudUnavailable')
                        : t('enterValidServerUrl')
                );
            }

            const authResult = await authenticateRemote({
                baseUrl,
                mode: syncAuthMode,
                loginEmail: syncLoginEmail,
                firstName: syncFirstName,
                lastName: syncLastName,
                email: syncEmail,
                password: syncPassword,
            });

            await saveRemoteSession(authResult);

            const normalizedCustomUrl = settings.sync_mode === 'CloudCustom'
                ? normalizeSyncBaseUrl(settings.custom_server_url)
                : settings.custom_server_url;
            const userName = formatRemoteUserName(authResult.user);
            const updatedSettings = {
                ...settings,
                custom_server_url: normalizedCustomUrl,
                sync_user_id: authResult.user.id,
                sync_user_name: userName,
            };

            if (settings.sync_mode === 'CloudCustom' && normalizedCustomUrl !== settings.custom_server_url) {
                await persistSettings(updatedSettings);
            } else {
                setSettings(updatedSettings);
            }

            setSyncSession({
                user_id: authResult.user.id,
                user_name: userName,
                token: authResult.token,
            });
            setSyncLoginEmail('');
            setSyncFirstName('');
            setSyncLastName('');
            setSyncEmail('');
            setSyncPassword('');
            notifySyncConfigUpdated();
        } catch (err) {
            console.error('Failed to authenticate sync account:', err);
            setSyncError(err instanceof Error ? err.message : t('remoteAuthFailed'));
        } finally {
            setSyncPending(false);
        }
    };

    const handleSignOut = async () => {
        if (!settings || syncPending) return;

        try {
            setSyncPending(true);
            await clearRemoteSession();
            setSettings({
                ...settings,
                sync_user_id: null,
                sync_user_name: null,
            });
            setSyncSession({ user_id: null, user_name: null, token: null });
            setSyncLoginEmail('');
            setSyncFirstName('');
            setSyncLastName('');
            setSyncPassword('');
            setSyncError(null);
            setVoiceCapability(null);
            setVoiceCapabilityError(null);
            notifySyncConfigUpdated();
        } catch (err) {
            console.error('Failed to clear sync session:', err);
        } finally {
            setSyncPending(false);
        }
    };

    const handleGoogleRemoteAuth = async () => {
        if (!settings || syncPending) return;

        try {
            setSyncPending(true);
            setSyncError(null);

            const baseUrl = resolveAuthBaseUrl(settings.sync_mode, settings.custom_server_url);

            if (!baseUrl) {
                throw new Error(
                    settings.sync_mode === 'CloudOfficial'
                        ? t('officialCloudUnavailable')
                        : t('enterValidServerUrl')
                );
            }

            if (settings.sync_mode === 'CloudCustom') {
                const normalizedCustomUrl = normalizeSyncBaseUrl(settings.custom_server_url);
                if (normalizedCustomUrl !== settings.custom_server_url) {
                    await persistSettings({
                        ...settings,
                        custom_server_url: normalizedCustomUrl,
                    });
                }
            }

            await startGoogleRemoteOAuth(baseUrl);
        } catch (err) {
            console.error('Failed to start sync Google OAuth:', err);
            setSyncError(err instanceof Error ? err.message : t('remoteAuthFailed'));
            setSyncPending(false);
        }
    };

    const handleVoiceModeChange = async (mode: UserSettings['voice']['mode']) => {
        if (!settings) return;
        const updated = {
            ...settings,
            voice: {
                ...settings.voice,
                mode,
            },
        };
        await persistSettings(updated);
    };

    const handleVoiceFieldChange = async (patch: Partial<UserSettings['voice']>) => {
        if (!settings) return;
        const updated = {
            ...settings,
            voice: {
                ...settings.voice,
                ...patch,
            },
        };
        await persistSettings(updated);
    };

    const handleSaveVoiceApiKey = async () => {
        if (!voiceApiKeyDraft.trim()) {
            return;
        }

        try {
            await invoke('save_voice_direct_api_key', { apiKey: voiceApiKeyDraft.trim() });
            setVoiceApiKeyDraft('');
            setVoiceSecretStatus({ direct_api_key_present: true });
        } catch (error) {
            console.error('Failed to save voice API key:', error);
        }
    };

    const handleClearVoiceApiKey = async () => {
        try {
            await invoke('clear_voice_direct_api_key');
            setVoiceApiKeyDraft('');
            setVoiceSecretStatus({ direct_api_key_present: false });
        } catch (error) {
            console.error('Failed to clear voice API key:', error);
        }
    };

    const isGeminiProvider = newProvider.kind === 'Gemini';
    const geminiModelOptions = (() => {
        const options = [...geminiModels];
        const currentModel = newProvider.model_name?.trim();
        if (currentModel && !options.some((model) => model.name === currentModel)) {
            options.unshift({ name: currentModel, label: currentModel });
        }
        return options;
    })();

    const handleCustomServerUrlCommit = async (value: string) => {
        if (!settings) return;

        const normalized = normalizeSyncBaseUrl(value);
        const didChange = normalized !== normalizeSyncBaseUrl(settings.custom_server_url);
        const updated = {
            ...settings,
            custom_server_url: normalized,
            sync_user_id: didChange ? null : settings.sync_user_id,
            sync_user_name: didChange ? null : settings.sync_user_name,
        };

        try {
            if (didChange) {
                await clearRemoteSession();
                setSyncSession({ user_id: null, user_name: null, token: null });
                setSyncLoginEmail('');
                setSyncPassword('');
                setSyncError(null);
            }

            await persistSettings(updated);

            if (didChange) {
                notifySyncConfigUpdated();
            }
        } catch (err) {
            console.error("Failed to save custom server URL:", err);
        }
    };

    const handleLocaleChange = async (locale: string) => {
        if (!settings) return;
        const updated = { ...settings, locale };
        await invoke("save_settings", { settings: updated });
        setSettings(updated);
        setTimeout(() => {
            window.dispatchEvent(new CustomEvent('localeUpdated'));
        }, 100);
    };

    const handleHourFormatChange = async (hour12: boolean) => {
        if (!settings) return;
        const updated = { ...settings, hour12 };
        await invoke("save_settings", { settings: updated });
        setSettings(updated);
        setTimeout(() => {
            window.dispatchEvent(new CustomEvent('localeUpdated'));
        }, 100);
    };

    const handleAddSavedLocation = async () => {
        if (!settings) return;

        const label = newSavedLocation.label.trim();
        const address = newSavedLocation.address.trim();
        if (!label || !address) {
            return;
        }

        const updated = {
            ...settings,
            saved_locations: [
                ...(settings.saved_locations || []),
                {
                    id: crypto.randomUUID(),
                    label,
                    location: {
                        location_type: 'address_text' as const,
                        address,
                        label,
                    },
                },
            ],
        };

        await persistSettings(updated);
        setNewSavedLocation({ label: '', address: '' });
    };

    const handleDeleteSavedLocation = async (locationId: string) => {
        if (!settings) return;

        const updated = {
            ...settings,
            saved_locations: (settings.saved_locations || []).filter((location) => location.id !== locationId),
        };

        await persistSettings(updated);
    };

    const handleNewSavedLocationChange = (field: 'label' | 'address', value: string) => {
        setNewSavedLocation((current) => ({ ...current, [field]: value }));
    };

    return (
        <AnimatePresence>
            {isOpen && (
                <motion.div 
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: 20 }}
                    className="absolute inset-0 z-[100] flex flex-col bg-[#080808] overflow-hidden"
                >
                    <WebGLGrain 
                        colors={{ c1: [20, 20, 20], c2: [15, 15, 15], c3: [10, 10, 10], c4: [5, 5, 5] }}
                        spreadX={0.35} spreadY={1.1} contrast={2.0} noiseFactor={0.7} opacity={1.0}
                    />

                    <div
                        className="relative z-20 h-full flex flex-col p-6 pt-12"
                        style={{
                            paddingTop: 'calc(48px + env(safe-area-inset-top, 0px))',
                            paddingBottom: 'calc(24px + env(safe-area-inset-bottom, 0px))',
                        }}
                    >
                        {/* Header */}
                        <div className="flex items-center justify-between mb-8 shrink-0" data-tauri-drag-region>
                            <div className="flex items-center gap-3 pointer-events-none" data-tauri-drag-region>
                                <div data-tauri-drag-region>
                                    <h2 className="text-[20px] font-semibold tracking-tight text-[#D1D1D1]" data-tauri-drag-region>{t('settingsTitle')}</h2>
                                    <p className="text-[9px] text-[#777] uppercase tracking-widest font-bold" data-tauri-drag-region>{t('settingsSubtitle')}</p>
                                </div>
                            </div>
                            <button 
                                onClick={onClose}
                                className="w-10 h-10 rounded-full hover:bg-white/5 flex items-center justify-center text-[#777] hover:text-[#D1D1D1] transition-all shrink-0"
                            >
                                <X size={24} />
                            </button>
                        </div>

                        {/* Scrollable Content Area */}
                        <div className="flex-1 overflow-y-auto no-scrollbar pr-2 pb-2">
                            <HostingSyncSection
                                settings={settings}
                                t={t}
                                hasRemoteMode={hasRemoteMode}
                                hasRemoteSession={hasRemoteSession}
                                remoteBaseUrl={remoteBaseUrl}
                                syncSession={syncSession}
                                syncAuthMode={syncAuthMode}
                                syncLoginEmail={syncLoginEmail}
                                syncFirstName={syncFirstName}
                                syncLastName={syncLastName}
                                syncEmail={syncEmail}
                                syncPassword={syncPassword}
                                syncPending={syncPending}
                                syncError={syncError}
                                onSyncModeChange={handleSyncModeChange}
                                onCustomServerUrlChange={(value) => settings && setSettings({ ...settings, custom_server_url: value })}
                                onCustomServerUrlCommit={handleCustomServerUrlCommit}
                                onSyncAuthModeChange={setSyncAuthMode}
                                onSyncLoginEmailChange={setSyncLoginEmail}
                                onSyncFirstNameChange={setSyncFirstName}
                                onSyncLastNameChange={setSyncLastName}
                                onSyncEmailChange={setSyncEmail}
                                onSyncPasswordChange={setSyncPassword}
                                onRemoteAuth={handleRemoteAuth}
                                onRemoteGoogleAuth={handleGoogleRemoteAuth}
                                onSignOut={handleSignOut}
                            />

                            <VoiceSection
                                settings={settings}
                                t={t}
                                voiceSecretStatus={voiceSecretStatus}
                                voiceCapability={voiceCapability}
                                voiceCapabilityError={voiceCapabilityError}
                                voiceApiKeyDraft={voiceApiKeyDraft}
                                onVoiceModeChange={handleVoiceModeChange}
                                onVoiceFieldChange={(patch) => {
                                    void handleVoiceFieldChange(patch);
                                }}
                                onVoiceApiKeyDraftChange={setVoiceApiKeyDraft}
                                onSaveVoiceApiKey={() => {
                                    void handleSaveVoiceApiKey();
                                }}
                                onClearVoiceApiKey={() => {
                                    void handleClearVoiceApiKey();
                                }}
                            />

                            <ProvidersSection
                                settings={settings}
                                t={t}
                                onAddNew={handleAddNew}
                                onSetDefault={setDefault}
                                onEditProvider={handleEditProvider}
                                onDeleteProvider={handleDeleteProvider}
                                isAdding={isAdding}
                            />

                            <LocaleSection
                                settings={settings}
                                t={t}
                                onLocaleChange={handleLocaleChange}
                                onHourFormatChange={handleHourFormatChange}
                            />

                            <SavedLocationsSection
                                settings={settings}
                                t={t}
                                newSavedLocation={newSavedLocation}
                                onNewSavedLocationChange={handleNewSavedLocationChange}
                                onAddSavedLocation={handleAddSavedLocation}
                                onDeleteSavedLocation={handleDeleteSavedLocation}
                            />
                        </div>

                        {/* Footer / Info - Fixed at bottom */}
                        <div className="mt-6 pt-6 border-t border-white/5 shrink-0">
                            <div className="flex items-center gap-3 p-4 rounded-[1.25rem] bg-[#141414] shadow-[inset_0_2px_5px_rgba(0,0,0,0.8)] border border-transparent text-[11px] text-[#777] leading-relaxed relative overflow-hidden">
                                <Key size={16} className="shrink-0 text-[#D1D1D1] relative z-10" />
                                <p className="relative z-10">{t('apiKeysSecurity')}</p>
                                <div className="absolute inset-0 bg-[#121212] opacity-50 z-0"></div>
                            </div>
                        </div>

                        <ProviderModal
                            isOpen={isAdding}
                            t={t}
                            newProvider={newProvider}
                            isGeminiProvider={isGeminiProvider}
                            geminiModelOptions={geminiModelOptions}
                            geminiModelsError={geminiModelsError}
                            isLoadingGeminiModels={isLoadingGeminiModels}
                            onClose={() => setIsAdding(false)}
                            onProviderFieldChange={updateNewProvider}
                            onProviderKindChange={handleProviderKindChange}
                            onRefreshGeminiModels={() => void fetchGeminiModels(newProvider.apiKey || '')}
                            onSaveProvider={handleUpsertProvider}
                        />
                    </div>
                </motion.div>
            )}
        </AnimatePresence>
    );
};
