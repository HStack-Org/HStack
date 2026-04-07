import type { ChangeEvent, FocusEvent, KeyboardEvent } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Database, Edit2, Globe, LoaderCircle, LogOut, Mic, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { isOfficialCloudConfigured, type SyncSessionInfo } from "../../syncConfig";
import type { RemoteAuthMode } from "../../syncAuth";
import { getSupportedLocale, SUPPORTED_LOCALE_OPTIONS, type TranslationKey } from "../../i18n";
import { OPENAI_DEFAULT_ENDPOINT, VOICE_DEFAULT_ENDPOINT, type GeminiModelSummary, type ProviderDraft, type SavedProvider, type UserSettings, type VoiceCapabilityResponse, type VoiceSecretStatus } from "./types";
import { HOSTING_OPTIONS, cn, EngravedInput, InsetSurface, PhysicalWrapper, THEMES } from "./ui";

type Translator = (key: TranslationKey, vars?: Record<string, string | number>) => string;

interface HostingSyncSectionProps {
  settings: UserSettings | null;
  t: Translator;
  hasRemoteMode: boolean;
  hasRemoteSession: boolean;
  remoteBaseUrl: string | null;
  syncSession: SyncSessionInfo | null;
  syncAuthMode: RemoteAuthMode;
  syncLoginEmail: string;
  syncFirstName: string;
  syncLastName: string;
  syncEmail: string;
  syncPassword: string;
  syncPending: boolean;
  syncError: string | null;
  onSyncModeChange: (syncMode: UserSettings["sync_mode"]) => void;
  onCustomServerUrlChange: (value: string) => void;
  onCustomServerUrlCommit: (value: string) => Promise<void>;
  onSyncAuthModeChange: (mode: RemoteAuthMode) => void;
  onSyncLoginEmailChange: (value: string) => void;
  onSyncFirstNameChange: (value: string) => void;
  onSyncLastNameChange: (value: string) => void;
  onSyncEmailChange: (value: string) => void;
  onSyncPasswordChange: (value: string) => void;
  onRemoteAuth: () => void;
  onRemoteGoogleAuth: () => void;
  onSignOut: () => void;
}

export const HostingSyncSection = ({
  settings,
  t,
  hasRemoteMode,
  hasRemoteSession,
  remoteBaseUrl,
  syncSession,
  syncAuthMode,
  syncLoginEmail,
  syncFirstName,
  syncLastName,
  syncEmail,
  syncPassword,
  syncPending,
  syncError,
  onSyncModeChange,
  onCustomServerUrlChange,
  onCustomServerUrlCommit,
  onSyncAuthModeChange,
  onSyncLoginEmailChange,
  onSyncFirstNameChange,
  onSyncLastNameChange,
  onSyncEmailChange,
  onSyncPasswordChange,
  onRemoteAuth,
  onRemoteGoogleAuth,
  onSignOut,
}: HostingSyncSectionProps) => (
  <section className="mb-8">
    <div className="mb-4 flex items-center justify-between px-1">
      <h3 className="flex items-center gap-2 text-[12px] font-bold uppercase tracking-widest text-[#777]">{t('settingsHostingSync')}</h3>
    </div>

    <div className="flex flex-col gap-3">
      {settings &&
        HOSTING_OPTIONS.map((option) => {
          const isSelected = settings.sync_mode === option.mode;
          const Icon = option.icon;

          return (
            <button key={option.mode} type="button" onClick={() => onSyncModeChange(option.mode)} className="text-left">
              <PhysicalWrapper
                outerClass="rounded-[1.15rem]"
                innerClass="flex items-start gap-3 px-3.5 py-3 transition-colors"
                shaderColors={isSelected ? THEMES.emerald : THEMES.default}
              >
                <div className="relative z-20 min-w-0 flex-1">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="flex items-center gap-2.5">
                        <div
                          className={cn(
                            "flex h-5 w-5 shrink-0 items-center justify-center transition-colors",
                            isSelected ? "text-[#D9DDE4]" : "text-[#C8CDD6]",
                          )}
                        >
                          <Icon size={16} />
                        </div>
                        <span className="text-[16px] font-medium tracking-[-0.02em] text-[#F1F1F1]">{t(option.titleKey)}</span>
                      </div>
                      <p className="mt-1 pr-1 text-[13px] leading-6 text-white/58">{t(option.descriptionKey)}</p>
                    </div>
                    <span
                      className={cn(
                        "inline-flex shrink-0 items-center pt-0.5 text-[9px] font-bold uppercase tracking-[0.22em]",
                        isSelected ? "text-white/45" : "text-white/32",
                      )}
                    >
                      {isSelected ? t('current') : t('select')}
                    </span>
                  </div>
                </div>
              </PhysicalWrapper>
            </button>
          );
        })}

      {settings?.sync_mode === 'CloudCustom' ? (
        <div className="pt-2">
          <EngravedInput
            label={t('customServerUrl')}
            value={settings.custom_server_url || ""}
            onChange={(event: ChangeEvent<HTMLInputElement>) => onCustomServerUrlChange(event.target.value)}
            onBlur={(event: FocusEvent<HTMLInputElement>) => {
              void onCustomServerUrlCommit(event.target.value);
            }}
            onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
              if (event.key === 'Enter') {
                event.currentTarget.blur();
              }
            }}
            placeholder={t('customServerPlaceholder')}
            className="font-mono"
          />
          <p className="mt-2 px-1 text-[10px] text-[#555]">{t('customServerHint')}</p>
        </div>
      ) : null}

      {hasRemoteMode && settings ? (
        <PhysicalWrapper outerClass="rounded-[1.15rem]" innerClass="flex flex-col gap-4 p-4" shaderColors={THEMES.default}>
          <div className="relative z-20 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <p className="text-[9px] font-bold uppercase tracking-[0.22em] text-white/32">{t('account')}</p>
              <p className="mt-2 text-[14px] leading-6 text-[#E6E6E6]">
                {hasRemoteSession
                  ? t('connectedAs', { name: syncSession?.user_name || settings.sync_user_name || t('yourAccount') })
                  : settings.sync_mode === 'CloudOfficial'
                    ? t('signInManagedCloud')
                    : t('signInSelfHosted')}
              </p>
            </div>
            {remoteBaseUrl ? (
              <span className="max-w-full self-start overflow-hidden text-ellipsis whitespace-nowrap rounded-full border border-white/10 bg-black/15 px-3 py-1 text-[10px] font-medium text-white/34 sm:max-w-[16rem]">
                {remoteBaseUrl}
              </span>
            ) : null}
          </div>

          {settings.sync_mode === 'CloudOfficial' && !isOfficialCloudConfigured() ? (
            <div className="relative z-20 rounded-[1rem] border border-amber-300/18 bg-amber-200/6 px-3.5 py-3 text-[12px] leading-5 text-amber-100/72">
              {t('officialCloudUnavailable')}
            </div>
          ) : null}

          {settings.sync_mode === 'CloudCustom' && !remoteBaseUrl ? (
            <div className="relative z-20 rounded-[1rem] border border-white/8 bg-black/18 px-3.5 py-3 text-[12px] leading-5 text-white/48">
              {t('enterServerBeforeSignIn')}
            </div>
          ) : null}

          {hasRemoteSession ? (
            <div className="relative z-20 flex items-center justify-between gap-4 rounded-[1rem] border border-white/8 bg-black/14 px-3.5 py-3.5">
              <div className="min-w-0">
                <p className="text-[15px] font-medium text-[#F1F1F1]">{syncSession?.user_name || settings.sync_user_name || t('connectedAccount')}</p>
                <p className="mt-1 text-[12px] text-white/42">
                  {settings.sync_mode === 'CloudOfficial' ? t('managedCloudSession') : t('selfHostedSession')}
                </p>
              </div>
              <button
                type="button"
                onClick={onSignOut}
                disabled={syncPending}
                className="inline-flex shrink-0 items-center gap-2 rounded-[999px] border border-white/10 px-3 py-2 text-[10px] font-bold uppercase tracking-[0.18em] text-white/58 transition-colors hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
              >
                {syncPending ? <LoaderCircle size={13} className="animate-spin" /> : <LogOut size={13} />}
                {t('signOut')}
              </button>
            </div>
          ) : (
            <div className="relative z-20 flex flex-col gap-4">
              <div className="flex rounded-[999px] border border-white/8 bg-black/15 p-1">
                {(['login', 'register'] as const).map((mode) => {
                  const isActive = syncAuthMode === mode;
                  return (
                    <button
                      key={mode}
                      type="button"
                      onClick={() => onSyncAuthModeChange(mode)}
                      className={cn(
                        "flex-1 rounded-[999px] px-3 py-2 text-[10px] font-bold uppercase tracking-[0.18em] transition-colors",
                        isActive ? "bg-white text-[#080808]" : "text-white/40 hover:text-white/64",
                      )}
                    >
                      {mode === 'login' ? t('login') : t('register')}
                    </button>
                  );
                })}
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <EngravedInput
                  label={t('email')}
                  value={syncAuthMode === 'login' ? syncLoginEmail : syncEmail}
                  onChange={(event: ChangeEvent<HTMLInputElement>) =>
                    syncAuthMode === 'login'
                      ? onSyncLoginEmailChange(event.target.value)
                      : onSyncEmailChange(event.target.value)
                  }
                  placeholder={t('enterEmail')}
                  type="email"
                />
                {syncAuthMode === 'register' ? (
                  <EngravedInput
                    label={t('firstName')}
                    value={syncFirstName}
                    onChange={(event: ChangeEvent<HTMLInputElement>) => onSyncFirstNameChange(event.target.value)}
                    placeholder={t('chooseFirstName')}
                    type="text"
                  />
                ) : (
                  <EngravedInput
                    label={t('password')}
                    type="password"
                    value={syncPassword}
                    onChange={(event: ChangeEvent<HTMLInputElement>) => onSyncPasswordChange(event.target.value)}
                    placeholder={t('enterPassword')}
                  />
                )}
              </div>

              {syncAuthMode === 'register' ? (
                <div className="grid gap-3 sm:grid-cols-2">
                  <EngravedInput
                    label={t('lastName')}
                    value={syncLastName}
                    onChange={(event: ChangeEvent<HTMLInputElement>) => onSyncLastNameChange(event.target.value)}
                    placeholder={t('optional')}
                  />
                  <EngravedInput
                    label={t('password')}
                    type="password"
                    value={syncPassword}
                    onChange={(event: ChangeEvent<HTMLInputElement>) => onSyncPasswordChange(event.target.value)}
                    placeholder={t('createPassword')}
                  />
                </div>
              ) : null}

              {syncError ? (
                <div className="rounded-[1rem] border border-red-400/18 bg-red-500/8 px-3.5 py-3 text-[12px] leading-5 text-red-100/80">
                  {syncError}
                </div>
              ) : null}

              <button
                type="button"
                onClick={onRemoteGoogleAuth}
                disabled={!remoteBaseUrl || syncPending}
                className="inline-flex items-center justify-center gap-2 rounded-[1rem] border border-white/10 bg-white/[0.04] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.2em] text-[#F3F3F3] transition-all hover:bg-white/[0.08] disabled:cursor-not-allowed disabled:opacity-45"
              >
                {syncPending ? <LoaderCircle size={14} className="animate-spin" /> : null}
                {t('continueWithGoogle')}
              </button>

              <div className="flex items-center gap-3 px-1">
                <div className="h-px flex-1 bg-white/8" />
                <span className="text-[9px] font-bold uppercase tracking-[0.22em] text-white/26">{t('or')}</span>
                <div className="h-px flex-1 bg-white/8" />
              </div>

              <button
                type="button"
                onClick={onRemoteAuth}
                disabled={
                  (syncAuthMode === 'login' ? !syncLoginEmail.trim() : !syncEmail.trim() || !syncFirstName.trim()) ||
                  !syncPassword ||
                  !remoteBaseUrl ||
                  syncPending
                }
                className="inline-flex items-center justify-center gap-2 rounded-[1rem] bg-[#EFEFEF] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.2em] text-[#080808] transition-all hover:bg-white disabled:cursor-not-allowed disabled:opacity-45"
              >
                {syncPending ? <LoaderCircle size={14} className="animate-spin" /> : null}
                {syncAuthMode === 'login' ? t('signIn') : t('createAccount')}
              </button>

              <p className="text-[10px] leading-5 text-white/26">{t('syncKeychainHint')}</p>
            </div>
          )}
        </PhysicalWrapper>
      ) : null}
    </div>
  </section>
);

interface ProvidersSectionProps {
  settings: UserSettings | null;
  t: Translator;
  onAddNew: () => void;
  onSetDefault: (id: string) => void;
  onEditProvider: (provider: SavedProvider) => void;
  onDeleteProvider: (id: string) => void;
  isAdding: boolean;
}

export const ProvidersSection = ({ settings, t, onAddNew, onSetDefault, onEditProvider, onDeleteProvider, isAdding }: ProvidersSectionProps) => (
  <section className="mb-8">
    <div className="mb-4 flex items-center justify-between px-1">
      <h3 className="flex items-center gap-2 text-[12px] font-bold uppercase tracking-widest text-[#777]">{t('llmProviders')}</h3>
      <button
        onClick={onAddNew}
        className="flex shrink-0 items-center gap-1.5 text-[9px] font-bold uppercase tracking-widest text-[#D1D1D1] transition-colors hover:text-white"
      >
        <Plus size={12} />
        {t('addNew')}
      </button>
    </div>

    <div className="flex flex-col gap-3">
      {settings?.providers.map((provider) => {
        const isDefault = settings.default_provider_id === provider.id;

        return (
          <PhysicalWrapper
            key={provider.id}
            innerClass="group flex cursor-pointer flex-col gap-3 p-4 transition-colors"
            shaderColors={isDefault ? THEMES.emerald : THEMES.default}
          >
            <div className="absolute inset-0 z-10" onClick={() => onSetDefault(provider.id)} />
            <div className="pointer-events-none relative z-20 flex items-start justify-between">
              <div className="min-w-0 flex flex-col gap-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-[15px] font-medium text-[#D1D1D1]">{provider.name}</span>
                </div>
                <span className="break-all font-mono text-[12px] tracking-tight text-[#777]">{provider.model_name}</span>
              </div>
              <div className="pointer-events-auto flex shrink-0 items-center gap-1">
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    onEditProvider(provider);
                  }}
                  className="rounded-lg p-2 text-[#777] opacity-0 transition-all hover:bg-white/5 hover:text-[#D1D1D1] group-hover:opacity-100"
                  title={t('editProvider')}
                >
                  <Edit2 size={16} />
                </button>
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    onDeleteProvider(provider.id);
                  }}
                  className="rounded-lg p-2 text-[#777] opacity-0 transition-all hover:bg-red-500/10 hover:text-red-400 group-hover:opacity-100"
                  title={t('deleteProvider')}
                >
                  <Trash2 size={16} />
                </button>
              </div>
            </div>
            <div className="pointer-events-none relative z-20 flex items-center gap-4 text-[11px] text-[#777]">
              <div className="flex shrink-0 items-center gap-1.5">
                <Database size={12} />
                {provider.kind === 'OpenAiCompatible' ? t('openAiCompatible') : t('googleGemini')}
              </div>
              <div className="flex min-w-0 items-center gap-1.5">
                <Globe size={12} className="shrink-0" />
                <span className="truncate">{provider.endpoint || t('cloudApi')}</span>
              </div>
            </div>
          </PhysicalWrapper>
        );
      })}

      {settings?.providers.length === 0 && !isAdding ? (
        <div className="rounded-[1.25rem] border border-dashed border-[#333] p-8 text-center">
          <p className="text-[13px] text-[#777]">{t('noProvidersConfigured')}</p>
        </div>
      ) : null}
    </div>
  </section>
);

interface VoiceSectionProps {
  settings: UserSettings | null;
  t: Translator;
  voiceSecretStatus: VoiceSecretStatus | null;
  voiceCapability: VoiceCapabilityResponse | null;
  voiceCapabilityError: string | null;
  voiceApiKeyDraft: string;
  onVoiceModeChange: (mode: UserSettings['voice']['mode']) => void;
  onVoiceFieldChange: (patch: Partial<UserSettings['voice']>) => void;
  onVoiceApiKeyDraftChange: (value: string) => void;
  onSaveVoiceApiKey: () => void;
  onClearVoiceApiKey: () => void;
}

export const VoiceSection = ({
  settings,
  t,
  voiceSecretStatus,
  voiceCapability,
  voiceCapabilityError,
  voiceApiKeyDraft,
  onVoiceModeChange,
  onVoiceFieldChange,
  onVoiceApiKeyDraftChange,
  onSaveVoiceApiKey,
  onClearVoiceApiKey,
}: VoiceSectionProps) => {
  const voiceSettings = settings?.voice;
  const modeOptions: Array<{ value: UserSettings['voice']['mode']; title: TranslationKey; description: TranslationKey }> = [
    { value: 'Disabled', title: 'voiceModeDisabledTitle', description: 'voiceModeDisabledDescription' },
    { value: 'Auto', title: 'voiceModeAutoTitle', description: 'voiceModeAutoDescription' },
    { value: 'DirectOnly', title: 'voiceModeDirectTitle', description: 'voiceModeDirectDescription' },
  ];

  return (
    <section className="mb-8">
      <div className="mb-4 flex items-center justify-between px-1">
        <h3 className="flex items-center gap-2 text-[12px] font-bold uppercase tracking-widest text-[#777]">{t('voiceInputTitle')}</h3>
      </div>

      <div className="flex flex-col gap-3">
        {modeOptions.map((option) => {
          const isSelected = voiceSettings?.mode === option.value;

          return (
            <button key={option.value} type="button" onClick={() => onVoiceModeChange(option.value)} className="text-left">
              <PhysicalWrapper
                outerClass="rounded-[1.15rem]"
                innerClass="flex items-start gap-3 px-3.5 py-3 transition-colors"
                shaderColors={isSelected ? THEMES.emerald : THEMES.default}
              >
                <div className="relative z-20 min-w-0 flex-1">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="flex items-center gap-2.5">
                        <div className={cn("flex h-5 w-5 shrink-0 items-center justify-center transition-colors", isSelected ? "text-[#D9DDE4]" : "text-[#C8CDD6]") }>
                          <Mic size={16} />
                        </div>
                        <span className="text-[16px] font-medium tracking-[-0.02em] text-[#F1F1F1]">{t(option.title)}</span>
                      </div>
                      <p className="mt-1 pr-1 text-[13px] leading-6 text-white/58">{t(option.description)}</p>
                    </div>
                    <span className={cn("inline-flex shrink-0 items-center pt-0.5 text-[9px] font-bold uppercase tracking-[0.22em]", isSelected ? "text-white/45" : "text-white/32") }>
                      {isSelected ? t('current') : t('select')}
                    </span>
                  </div>
                </div>
              </PhysicalWrapper>
            </button>
          );
        })}

        <PhysicalWrapper outerClass="rounded-[1.15rem]" innerClass="flex flex-col gap-4 p-4" shaderColors={THEMES.default}>
          <div className="relative z-20 flex flex-col gap-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <EngravedInput
                label={t('voiceEndpointLabel')}
                value={voiceSettings?.direct_api_base_url || VOICE_DEFAULT_ENDPOINT}
                onChange={(event: ChangeEvent<HTMLInputElement>) => onVoiceFieldChange({ direct_api_base_url: event.target.value })}
                placeholder={VOICE_DEFAULT_ENDPOINT}
              />
              <EngravedInput
                label={t('voiceModelLabel')}
                value={voiceSettings?.direct_model_name || ''}
                onChange={(event: ChangeEvent<HTMLInputElement>) => onVoiceFieldChange({ direct_model_name: event.target.value })}
                placeholder="voxtral-mini-transcribe-realtime-2602"
              />
            </div>

            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-end">
              <EngravedInput
                label={t('voiceApiKeyLabel')}
                type="password"
                value={voiceApiKeyDraft}
                onChange={(event: ChangeEvent<HTMLInputElement>) => onVoiceApiKeyDraftChange(event.target.value)}
                placeholder={voiceSecretStatus?.direct_api_key_present ? t('apiKeyPlaceholderKeepExisting') : t('voiceApiKeyPlaceholder')}
              />
              <button
                type="button"
                onClick={onSaveVoiceApiKey}
                disabled={!voiceApiKeyDraft.trim()}
                className="inline-flex items-center justify-center rounded-[1rem] bg-[#EFEFEF] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.2em] text-[#080808] transition-all hover:bg-white disabled:cursor-not-allowed disabled:opacity-45"
              >
                {t('saveApiKey')}
              </button>
              <button
                type="button"
                onClick={onClearVoiceApiKey}
                disabled={!voiceSecretStatus?.direct_api_key_present}
                className="inline-flex items-center justify-center rounded-[1rem] border border-white/10 px-4 py-3 text-[11px] font-bold uppercase tracking-[0.2em] text-white/58 transition-all hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
              >
                {t('clearApiKey')}
              </button>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <EngravedInput
                label={t('voiceTargetDelayLabel')}
                type="number"
                min={0}
                step={10}
                value={voiceSettings?.target_streaming_delay_ms ?? ''}
                onChange={(event: ChangeEvent<HTMLInputElement>) => {
                  const value = event.target.value.trim();
                  onVoiceFieldChange({ target_streaming_delay_ms: value ? Number(value) : null });
                }}
                placeholder="240"
              />
              <InsetSurface className="px-4 py-3">
                <div className="text-[9px] font-bold uppercase tracking-widest text-[#777]">{t('managedVoiceStatus')}</div>
                <div className="mt-2 text-[14px] text-[#E6E6E6]">
                  {voiceCapability?.available ? t('managedVoiceAvailable') : t('managedVoiceUnavailable')}
                </div>
                <div className="mt-1 text-[12px] leading-5 text-white/42">
                  {voiceCapabilityError
                    ? voiceCapabilityError
                    : voiceCapability?.reason
                      ? t('managedVoiceReason', { reason: voiceCapability.reason })
                      : t('managedVoiceHint')}
                </div>
              </InsetSurface>
            </div>
          </div>
        </PhysicalWrapper>
      </div>
    </section>
  );
};

interface LocaleSectionProps {
  settings: UserSettings | null;
  t: Translator;
  onLocaleChange: (locale: string) => Promise<void>;
  onHourFormatChange: (hour12: boolean) => Promise<void>;
}

export const LocaleSection = ({ settings, t, onLocaleChange, onHourFormatChange }: LocaleSectionProps) => {
  const selectedLocale = getSupportedLocale(settings?.locale || 'en-GB');

  return (
    <section className="mt-6 border-t border-white/5 pt-6">
      <div className="mb-4 flex items-center justify-between px-1">
        <h3 className="flex items-center gap-2 text-[12px] font-bold uppercase tracking-widest text-[#777]">{t('localeDisplay')}</h3>
      </div>

      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <label className="px-1 text-[9px] font-bold uppercase tracking-widest text-[#777]">{t('languageRegion')}</label>
          <select
            value={selectedLocale}
            onChange={(event) => {
              void onLocaleChange(event.target.value);
            }}
            className="rounded-[1.25rem] border border-transparent bg-[#141414] p-4 text-[14px] text-[#D1D1D1] outline-none shadow-[inset_0_2px_5px_rgba(0,0,0,0.8)] transition-all hover:border-white/5"
          >
            {SUPPORTED_LOCALE_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.value === 'en-GB'
                  ? t('languageEnglishUk')
                  : option.value === 'fr-FR'
                    ? t('languageFrench')
                    : option.value === 'es-ES'
                      ? t('languageSpanish')
                      : t('languageChinese')}
              </option>
            ))}
          </select>
          <p className="px-1 text-[10px] text-[#555]">{t('localeAppliesImmediately')}</p>
        </div>

        <div className="flex flex-col gap-2">
          <label className="px-1 text-[9px] font-bold uppercase tracking-widest text-[#777]">{t('timeFormat')}</label>
          <div className="flex gap-2 rounded-[1.25rem] border border-transparent bg-[#141414] p-[4px] shadow-[inset_0_2px_5px_rgba(0,0,0,0.8)]">
            <button
              onClick={() => {
                void onHourFormatChange(true);
              }}
              className={cn(
                "relative flex-1 overflow-hidden rounded-[13px] py-3 text-[12px] font-medium transition-all",
                settings?.hour12 !== false ? "text-[#D1D1D1] shadow-[0_2px_5px_rgba(0,0,0,0.7)]" : "bg-transparent text-[#777] hover:text-[#D1D1D1]",
              )}
            >
              {settings?.hour12 !== false ? (
                <div className="absolute inset-0 rounded-[13px] bg-[linear-gradient(180deg,#1c1c1c_0%,#141414_100%)]" />
              ) : null}
              <span className="relative z-10">{t('timeFormat12h')}</span>
            </button>
            <button
              onClick={() => {
                void onHourFormatChange(false);
              }}
              className={cn(
                "relative flex-1 overflow-hidden rounded-[13px] py-3 text-[12px] font-medium transition-all",
                settings?.hour12 === false ? "text-[#D1D1D1] shadow-[0_2px_5px_rgba(0,0,0,0.7)]" : "bg-transparent text-[#777] hover:text-[#D1D1D1]",
              )}
            >
              {settings?.hour12 === false ? (
                <div className="absolute inset-0 rounded-[13px] bg-[linear-gradient(180deg,#1c1c1c_0%,#141414_100%)]" />
              ) : null}
              <span className="relative z-10">{t('timeFormat24h')}</span>
            </button>
          </div>
          <p className="px-1 text-[10px] text-[#555]">{t('localeAppliesImmediately')}</p>
        </div>
      </div>
    </section>
  );
};

interface SavedLocationsSectionProps {
  settings: UserSettings | null;
  t: Translator;
  newSavedLocation: { label: string; address: string };
  onNewSavedLocationChange: (field: 'label' | 'address', value: string) => void;
  onAddSavedLocation: () => void;
  onDeleteSavedLocation: (locationId: string) => void;
}

export const SavedLocationsSection = ({
  settings,
  t,
  newSavedLocation,
  onNewSavedLocationChange,
  onAddSavedLocation,
  onDeleteSavedLocation,
}: SavedLocationsSectionProps) => (
  <section className="mt-6 border-t border-white/5 pt-6">
    <div className="mb-4 flex items-center justify-between px-1">
      <h3 className="flex items-center gap-2 text-[12px] font-bold uppercase tracking-widest text-[#777]">{t('savedLocationsTitle')}</h3>
    </div>

    <div className="flex flex-col gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <EngravedInput
          label={t('savedLocationLabel')}
          value={newSavedLocation.label}
          onChange={(event: ChangeEvent<HTMLInputElement>) => onNewSavedLocationChange('label', event.target.value)}
          placeholder={t('savedLocationLabelPlaceholder')}
        />
        <EngravedInput
          label={t('savedLocationAddress')}
          value={newSavedLocation.address}
          onChange={(event: ChangeEvent<HTMLInputElement>) => onNewSavedLocationChange('address', event.target.value)}
          placeholder={t('savedLocationAddressPlaceholder')}
        />
      </div>

      <button
        type="button"
        onClick={onAddSavedLocation}
        disabled={!newSavedLocation.label.trim() || !newSavedLocation.address.trim()}
        className="inline-flex items-center justify-center gap-2 rounded-[1rem] bg-[#EFEFEF] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.2em] text-[#080808] transition-all hover:bg-white disabled:cursor-not-allowed disabled:opacity-45"
      >
        <Plus size={14} />
        {t('saveLocation')}
      </button>

      <div className="flex flex-col gap-3">
        {(settings?.saved_locations || []).map((savedLocation) => (
          <PhysicalWrapper key={savedLocation.id} innerClass="flex items-start justify-between gap-3 p-4" shaderColors={THEMES.default}>
            <div className="relative z-20 min-w-0">
              <div className="truncate text-[15px] font-medium text-[#D1D1D1]">{savedLocation.label}</div>
              <div className="mt-1 break-words text-[12px] leading-5 text-white/48">{savedLocation.location.address}</div>
            </div>
            <button
              type="button"
              onClick={() => onDeleteSavedLocation(savedLocation.id)}
              className="relative z-20 shrink-0 rounded-lg p-2 text-[#777] transition-all hover:bg-red-500/10 hover:text-red-400"
              title={t('removeSavedLocation')}
            >
              <Trash2 size={16} />
            </button>
          </PhysicalWrapper>
        ))}

        {(settings?.saved_locations || []).length === 0 ? (
          <div className="rounded-[1.25rem] border border-dashed border-[#333] p-6 text-center">
            <p className="text-[13px] text-[#777]">{t('noSavedLocations')}</p>
          </div>
        ) : null}
      </div>
    </div>
  </section>
);

interface ProviderModalProps {
  isOpen: boolean;
  t: Translator;
  newProvider: ProviderDraft;
  isGeminiProvider: boolean;
  geminiModelOptions: GeminiModelSummary[];
  geminiModelsError: string | null;
  isLoadingGeminiModels: boolean;
  onClose: () => void;
  onProviderFieldChange: (patch: Partial<ProviderDraft>) => void;
  onProviderKindChange: (kind: SavedProvider['kind']) => void;
  onRefreshGeminiModels: () => void;
  onSaveProvider: () => void;
}

export const ProviderModal = ({
  isOpen,
  t,
  newProvider,
  isGeminiProvider,
  geminiModelOptions,
  geminiModelsError,
  isLoadingGeminiModels,
  onClose,
  onProviderFieldChange,
  onProviderKindChange,
  onRefreshGeminiModels,
  onSaveProvider,
}: ProviderModalProps) => (
  <AnimatePresence>
    {isOpen ? (
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="absolute inset-0 z-[110] flex flex-col bg-[#080808]/90 p-6 pt-12 backdrop-blur-md"
      >
        <div className="mb-8 flex items-center justify-between">
          <h3 className="text-[18px] font-semibold text-[#D1D1D1]">{newProvider.id ? t('editProvider') : t('newProvider')}</h3>
          <button onClick={onClose} className="text-[#777] transition-colors hover:text-[#D1D1D1]">
            <X size={24} />
          </button>
        </div>

        <div className="no-scrollbar flex flex-1 flex-col gap-5 overflow-y-auto pb-8">
          <EngravedInput
            label={t('friendlyName')}
            value={newProvider.name || ''}
            onChange={(event: ChangeEvent<HTMLInputElement>) => onProviderFieldChange({ name: event.target.value })}
            placeholder={t('friendlyNamePlaceholder')}
          />

          <div className="flex flex-col gap-2">
            <label className="px-1 text-[9px] font-bold uppercase tracking-widest text-[#777]">{t('providerKind')}</label>
            <div className="flex gap-2 rounded-[1.25rem] bg-[#141414] p-[4px] shadow-[inset_0_2px_5px_rgba(0,0,0,0.8)]">
              {(['OpenAiCompatible', 'Gemini'] as const).map((kind) => {
                const isSelected = newProvider.kind === kind;
                return (
                  <button
                    key={kind}
                    onClick={() => onProviderKindChange(kind)}
                    className={cn(
                      "relative flex-1 overflow-hidden rounded-[13px] py-3 text-[12px] font-medium transition-all",
                      isSelected ? "text-[#D1D1D1] shadow-[0_2px_5px_rgba(0,0,0,0.7)]" : "bg-transparent text-[#777] hover:text-[#D1D1D1]",
                    )}
                  >
                    {isSelected ? (
                      <div className="absolute inset-0 rounded-[13px] bg-[linear-gradient(180deg,#1c1c1c_0%,#141414_100%)]" />
                    ) : null}
                    <span className="relative z-10">{kind === 'OpenAiCompatible' ? t('openAiCompatible') : t('googleGemini')}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {isGeminiProvider ? (
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between gap-3 px-1">
                <label className="text-[9px] font-bold uppercase tracking-widest text-[#777]">{t('geminiModel')}</label>
                <button
                  type="button"
                  onClick={onRefreshGeminiModels}
                  disabled={!newProvider.apiKey?.trim() || isLoadingGeminiModels}
                  className="inline-flex items-center gap-1 text-[9px] font-bold uppercase tracking-widest text-[#777] transition-colors hover:text-[#D1D1D1] disabled:cursor-not-allowed disabled:opacity-40"
                >
                  <RefreshCw size={10} className={cn(isLoadingGeminiModels && 'animate-spin')} />
                  {t('refresh')}
                </button>
              </div>
              <InsetSurface>
                <select
                  value={newProvider.model_name || ''}
                  onChange={(event) => onProviderFieldChange({ model_name: event.target.value })}
                  disabled={geminiModelOptions.length === 0}
                  className="relative z-20 w-full bg-transparent px-4 py-3 text-[14px] text-[#D1D1D1] outline-none disabled:text-[#666]"
                >
                  {geminiModelOptions.length === 0 ? (
                    <option value="">{newProvider.apiKey?.trim() ? t('noGeminiModelsLoaded') : t('enterGeminiApiKeyFirst')}</option>
                  ) : null}
                  {geminiModelOptions.map((model) => (
                    <option key={model.name} value={model.name}>
                      {model.label}
                    </option>
                  ))}
                </select>
              </InsetSurface>
              <p className="px-1 text-[10px] text-[#555]">{t('geminiEndpointHint')}</p>
              {geminiModelsError ? <p className="px-1 text-[10px] text-red-300/70">{geminiModelsError}</p> : null}
            </div>
          ) : (
            <>
              <EngravedInput
                label={t('endpointUrl')}
                value={newProvider.endpoint || ''}
                onChange={(event: ChangeEvent<HTMLInputElement>) => onProviderFieldChange({ endpoint: event.target.value })}
                placeholder={OPENAI_DEFAULT_ENDPOINT}
                className="font-mono"
              />

              <EngravedInput
                label={t('modelName')}
                value={newProvider.model_name || ''}
                onChange={(event: ChangeEvent<HTMLInputElement>) => onProviderFieldChange({ model_name: event.target.value })}
                placeholder={t('modelNamePlaceholder')}
                className="font-mono"
              />
            </>
          )}

          <EngravedInput
            label={isGeminiProvider ? t('geminiApiKey') : t('apiKey')}
            type="password"
            value={newProvider.apiKey || ''}
            onChange={(event: ChangeEvent<HTMLInputElement>) => onProviderFieldChange({ apiKey: event.target.value })}
            placeholder={newProvider.id ? t('apiKeyPlaceholderKeepExisting') : '••••••••'}
          />

          <button
            onClick={onSaveProvider}
            className="mt-6 rounded-[1.25rem] bg-[#D1D1D1] py-4 text-[11px] font-bold uppercase tracking-widest text-[#080808] shadow-[0_4px_10px_rgba(0,0,0,0.5)] transition-all hover:scale-[1.02] active:scale-[0.98]"
          >
            {t('saveProvider')}
          </button>
        </div>
      </motion.div>
    ) : null}
  </AnimatePresence>
);