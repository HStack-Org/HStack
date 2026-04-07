import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { HardDrive, LoaderCircle, Server } from 'lucide-react';
import { WebGLGrain } from './WebGLGrain';
import { AnimatedWebGLGrain } from './AnimatedWebGLGrain';
import {
  REMOTE_GOOGLE_PROVIDER,
  REMOTE_OAUTH_REDIRECT_EVENT,
  authenticateRemote,
  clearRemoteSession,
  completeRemoteOAuth,
  parseRemoteOAuthRedirectUrl,
  saveRemoteSession,
  startGoogleRemoteOAuth,
  type RemoteAuthMode,
  resolveAuthBaseUrl,
} from '../syncAuth';
import { isOfficialCloudConfigured, normalizeSyncBaseUrl, notifySyncConfigUpdated, type SyncMode, type UserSettingsShape } from '../syncConfig';
import { useI18n } from '../i18n';

const blendToward = (color: number[], target: [number, number, number], amount: number) =>
  color.map((value, index) => Math.round(value + (target[index] - value) * amount));

const BACKGROUND_GRAIN_THEME = {
  c1: [30, 30, 30],
  c2: [12, 12, 12],
  c3: [9, 9, 9],
  c4: [6, 6, 6],
};

const BACKGROUND_GRAIN_THEME_COOL = {
  c1: [52, 64, 90],
  c2: [18, 20, 26],
  c3: [10, 10, 12],
  c4: [6, 6, 7],
};

const makeCardTheme = (theme: { c1: number[]; c2: number[]; c3: number[]; c4: number[] }) => ({
  c1: blendToward(theme.c1, [40, 40, 42], 0.42),
  c2: blendToward(theme.c2, [14, 14, 15], 0.28),
  c3: blendToward(theme.c3, [9, 9, 10], 0.14),
  c4: blendToward(theme.c4, [6, 6, 7], 0.08),
});

const CARD_GRAIN_THEME = makeCardTheme(BACKGROUND_GRAIN_THEME);
const CARD_GRAIN_THEME_COOL = makeCardTheme(BACKGROUND_GRAIN_THEME_COOL);

const HStackMark = ({ size = 17 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 210 210" fill="none" aria-hidden="true">
    <rect x="0" y="0" width="60" height="210" fill="currentColor" />
    <rect x="150" y="0" width="60" height="210" fill="currentColor" />
    <rect x="50" y="45" width="100" height="30" fill="currentColor" />
    <rect x="50" y="90" width="100" height="30" fill="currentColor" />
    <rect x="50" y="135" width="100" height="30" fill="currentColor" />
  </svg>
);

const WizardOptionCard = ({ children, topBorderColor, grainTheme, animatedPalette }: { children: React.ReactNode; topBorderColor: string; grainTheme: { c1: number[]; c2: number[]; c3: number[]; c4: number[] }; animatedPalette?: { from: { c1: number[]; c2: number[]; c3: number[]; c4: number[] }; to: { c1: number[]; c2: number[]; c3: number[]; c4: number[] }; durationMs?: number } }) => (
  <div className="relative rounded-[1.15rem] p-[0.5px] transition-all duration-300">
    <div
      className="absolute inset-0 rounded-[1.15rem]"
      style={{
        background: `linear-gradient(180deg, ${topBorderColor} 0%, rgba(104,128,186,0.065) 8%, rgba(255,255,255,0.014) 24%, rgba(255,255,255,0.012) 76%, rgba(96,118,176,0.04) 90%, rgba(110,134,192,0.07) 100%)`
      }}
    />
    <div className="relative overflow-hidden rounded-[calc(1.15rem-0.5px)] bg-[#17181b]">
      {animatedPalette ? (
        <AnimatedWebGLGrain colors={grainTheme} animatedPalette={animatedPalette} />
      ) : (
        <WebGLGrain colors={grainTheme} />
      )}
      <div className="absolute top-0 left-0 right-0 z-10 h-px bg-white/[0.012]" />
      <div className="absolute top-0 left-0 bottom-0 z-10 w-px bg-white/[0.012]" />
      <div className="relative z-10 h-full w-full">{children}</div>
    </div>
  </div>
);

interface SetupWizardProps {
  onComplete: () => void;
}

type SetupSettings = UserSettingsShape & Record<string, unknown>;

const WizardInput = ({
  label,
  className = '',
  ...props
}: React.InputHTMLAttributes<HTMLInputElement> & { label: string }) => (
  <label className="flex flex-col gap-2">
    <span className="px-1 text-[9px] font-bold uppercase tracking-[0.22em] text-white/32">{label}</span>
    <div className="rounded-[1rem] border border-white/8 bg-black/20 px-3.5 py-3 transition-colors focus-within:border-white/14">
      <input
        {...props}
        className={`w-full bg-transparent text-[14px] text-[#ECECEC] outline-none placeholder:text-white/24 ${className}`}
      />
    </div>
  </label>
);

export const SetupWizard: React.FC<SetupWizardProps> = ({ onComplete }) => {
  const { t } = useI18n();
  const [savingMode, setSavingMode] = useState<string | null>(null);
  const [selectedMode, setSelectedMode] = useState<SyncMode | null>(null);
  const [authMode, setAuthMode] = useState<RemoteAuthMode>('login');
  const [loginEmail, setLoginEmail] = useState('');
  const [firstName, setFirstName] = useState('');
  const [lastName, setLastName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [customServerUrl, setCustomServerUrl] = useState('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const selectedBaseUrl = selectedMode ? resolveAuthBaseUrl(selectedMode, customServerUrl) : null;
  const officialCloudReady = isOfficialCloudConfigured();
  const isRemoteMode = selectedMode === 'CloudOfficial' || selectedMode === 'CloudCustom';
  const hasLoginEmail = loginEmail.trim().length > 0;
  const hasRegisterFirstName = firstName.trim().length > 0;
  const hasRegisterEmail = email.trim().length > 0;
  const canSubmitRemote = Boolean(
    password.trim() &&
      selectedBaseUrl &&
      (authMode === 'login' ? hasLoginEmail : hasRegisterFirstName && hasRegisterEmail)
  );

  const options = [
    {
      mode: 'LocalOnly',
      title: t('hostingLocalTitle'),
      description: t('setupLocalDescription'),
      icon: HardDrive,
    },
    {
      mode: 'CloudOfficial',
      title: t('hostingOfficialTitle'),
      description: t('hostingOfficialDescription'),
      icon: HStackMark,
    },
    {
      mode: 'CloudCustom',
      title: t('hostingCustomTitle'),
      description: t('hostingCustomDescription'),
      icon: Server,
    },
  ] as const;

  useEffect(() => {
    const loadExistingSettings = async () => {
      try {
        const settings = await invoke<UserSettingsShape>('get_settings');
        setCustomServerUrl(settings.custom_server_url || '');
      } catch (error) {
        console.error('Failed to preload sync settings:', error);
      }
    };

    void loadExistingSettings();
  }, []);

  useEffect(() => {
    if (!selectedMode || !isRemoteMode) {
      return;
    }

    const handleOAuthRedirect = async (event: Event) => {
      const urls = (event as CustomEvent<string[]>).detail || [];
      const baseUrl = resolveAuthBaseUrl(selectedMode, customServerUrl);
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
        setErrorMessage(redirect.errorDescription || redirect.error);
        setSavingMode(null);
        return;
      }

      if (!redirect.code) {
        setErrorMessage(t('googleAuthMissingCode'));
        setSavingMode(null);
        return;
      }

      try {
        setSavingMode(selectedMode);
        setErrorMessage(null);

        const session = await completeRemoteOAuth(baseUrl, redirect.code);
        await saveRemoteSession(session);
        await invoke('complete_onboarding', { mode: selectedMode });
        notifySyncConfigUpdated();
        onComplete();
      } catch (error) {
        console.error('Failed to complete Google OAuth sign-in:', error);
        setErrorMessage(error instanceof Error ? error.message : t('remoteAuthFailed'));
        setSavingMode(null);
      }
    };

    window.addEventListener(REMOTE_OAUTH_REDIRECT_EVENT, handleOAuthRedirect as EventListener);
    return () => {
      window.removeEventListener(REMOTE_OAUTH_REDIRECT_EVENT, handleOAuthRedirect as EventListener);
    };
  }, [customServerUrl, isRemoteMode, onComplete, selectedMode, t]);

  const resetRemoteForm = () => {
    setSelectedMode(null);
    setAuthMode('login');
    setLoginEmail('');
    setFirstName('');
    setLastName('');
    setEmail('');
    setPassword('');
    setErrorMessage(null);
    setSavingMode(null);
  };

  const handleSelectMode = async (mode: string) => {
    if (savingMode) return;

    if (mode === 'CloudOfficial' || mode === 'CloudCustom') {
      setSelectedMode(mode);
      setAuthMode('login');
      setErrorMessage(null);
      return;
    }

    try {
      setSavingMode(mode);
      await clearRemoteSession();
      await invoke('complete_onboarding', { mode });
      notifySyncConfigUpdated();
      onComplete();
    } catch (error) {
      console.error('Failed to save hosting mode:', error);
      setSavingMode(null);
    }
  };

  const handleAuthenticate = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    if (!selectedMode || !isRemoteMode || savingMode) {
      return;
    }

    try {
      setSavingMode(selectedMode);
      setErrorMessage(null);

      const baseUrl = resolveAuthBaseUrl(selectedMode, customServerUrl);

      if (!baseUrl) {
        throw new Error(
          selectedMode === 'CloudOfficial'
            ? t('officialCloudNotReady')
            : t('enterValidBaseUrl')
        );
      }

      if (selectedMode === 'CloudCustom') {
        const currentSettings = await invoke<SetupSettings>('get_settings');
        await invoke('save_settings', {
          settings: {
            ...currentSettings,
            custom_server_url: normalizeSyncBaseUrl(customServerUrl),
          },
        });
      }

      const session = await authenticateRemote({
        baseUrl,
        mode: authMode,
        loginEmail,
        firstName,
        lastName,
        email,
        password,
      });

      await saveRemoteSession(session);
      await invoke('complete_onboarding', { mode: selectedMode });
      notifySyncConfigUpdated();
      onComplete();
    } catch (error) {
      console.error('Failed to authenticate hosted sync:', error);
      setErrorMessage(error instanceof Error ? error.message : t('remoteAuthFailed'));
      setSavingMode(null);
    }
  };

  const handleGoogleAuthenticate = async () => {
    if (!selectedMode || !isRemoteMode || savingMode) {
      return;
    }

    try {
      setSavingMode(selectedMode);
      setErrorMessage(null);

      const baseUrl = resolveAuthBaseUrl(selectedMode, customServerUrl);

      if (!baseUrl) {
        throw new Error(
          selectedMode === 'CloudOfficial'
            ? t('officialCloudNotReady')
            : t('enterValidBaseUrl')
        );
      }

      if (selectedMode === 'CloudCustom') {
        const currentSettings = await invoke<SetupSettings>('get_settings');
        await invoke('save_settings', {
          settings: {
            ...currentSettings,
            custom_server_url: normalizeSyncBaseUrl(customServerUrl),
          },
        });
      }

      await startGoogleRemoteOAuth(baseUrl);
    } catch (error) {
      console.error('Failed to start Google OAuth:', error);
      setErrorMessage(error instanceof Error ? error.message : t('remoteAuthFailed'));
      setSavingMode(null);
    }
  };

  const topBorderColor = 'rgba(108, 132, 192, 0.34)';
  const sharedAnimation = {
    from: CARD_GRAIN_THEME,
    to: CARD_GRAIN_THEME_COOL,
    durationMs: 5600,
  };

  const authTitle =
    selectedMode === 'CloudOfficial'
      ? t('signInOfficialCloud')
      : t('connectSelfHosted');

  const authDescription =
    selectedMode === 'CloudOfficial'
      ? t('signInOfficialCloudDescription')
      : t('connectSelfHostedDescription');

  return (
    <div className="absolute inset-0 z-[1000] overflow-hidden rounded-[inherit] bg-[#0b0b0c]">
      <div className="relative flex h-full flex-col overflow-hidden rounded-[inherit]">
        <AnimatedWebGLGrain
          colors={BACKGROUND_GRAIN_THEME}
          spreadX={0.35}
          spreadY={1.1}
          contrast={2.0}
          noiseFactor={0.7}
          opacity={1.0}
          animatedPalette={{
            from: BACKGROUND_GRAIN_THEME,
            to: BACKGROUND_GRAIN_THEME_COOL,
            durationMs: 5600,
          }}
        />

        <div
          className="absolute inset-0 z-10"
          style={{
            background: 'linear-gradient(180deg, rgba(9,10,12,0.04) 0%, rgba(6,6,7,0.22) 26%, rgba(4,4,5,0.58) 58%, rgba(2,2,3,0.88) 100%)'
          }}
        />

        <div
          className="relative z-20 flex h-full flex-col px-4 pb-4 pt-3 sm:px-5"
          style={{
            paddingTop: 'calc(12px + env(safe-area-inset-top, 0px))',
            paddingBottom: 'calc(16px + env(safe-area-inset-bottom, 0px))',
          }}
        >
          <div className="shrink-0 border-b border-white/6 px-1 pb-4 pt-1">
            <div className="flex items-center justify-between">
              {selectedMode ? (
                <button
                  type="button"
                  onClick={resetRemoteForm}
                  className="text-[11px] font-medium uppercase tracking-[0.22em] text-white/34 transition-colors hover:text-white/56"
                >
                  {t('back')}
                </button>
              ) : (
                <span className="text-[11px] font-medium uppercase tracking-[0.22em] text-white/24" data-tauri-drag-region>
                  {t('setup')}
                </span>
              )}
              <span className="h-4 w-12" data-tauri-drag-region />
            </div>
            <h1 className="mt-4 max-w-none pr-3 text-[1.95rem] font-semibold leading-[0.94] tracking-[-0.05em] text-[#F5F5F5]" data-tauri-drag-region>
              {selectedMode ? authTitle : t('setupQuestion')}
            </h1>
            <p className="mt-3 max-w-none pr-5 text-[13px] leading-5 text-white/54" data-tauri-drag-region>
              {selectedMode ? authDescription : t('setupDescription')}
            </p>
          </div>

          <div className="flex min-h-0 flex-1 flex-col pt-3">
            {selectedMode && isRemoteMode ? (
              <WizardOptionCard topBorderColor={topBorderColor} grainTheme={CARD_GRAIN_THEME} animatedPalette={sharedAnimation}>
                <form className="flex h-full flex-col gap-4 px-3.5 py-3.5" onSubmit={handleAuthenticate}>
                  <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                    <div className="min-w-0">
                      <p className="text-[9px] font-bold uppercase tracking-[0.22em] text-white/30">
                        {selectedMode === 'CloudOfficial' ? t('managedCloud') : t('customServer')}
                      </p>
                      {selectedBaseUrl ? (
                        <p className="mt-1 overflow-hidden text-ellipsis whitespace-nowrap text-[12px] text-white/46">{selectedBaseUrl}</p>
                      ) : null}
                    </div>

                    <div className="flex self-start rounded-[999px] border border-white/8 bg-black/15 p-1">
                      {(['login', 'register'] as const).map((mode) => {
                        const isActive = authMode === mode;
                        return (
                          <button
                            key={mode}
                            type="button"
                            onClick={() => setAuthMode(mode)}
                            className={`rounded-[999px] px-3 py-1.5 text-[10px] font-bold uppercase tracking-[0.18em] transition-colors ${
                              isActive ? 'bg-white text-[#080808]' : 'text-white/40 hover:text-white/62'
                            }`}
                          >
                            {mode === 'login' ? t('login') : t('register')}
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  {selectedMode === 'CloudOfficial' && !officialCloudReady ? (
                    <div className="rounded-[1rem] border border-amber-300/18 bg-amber-200/6 px-3.5 py-3 text-[12px] leading-5 text-amber-100/72">
                      {t('officialCloudNotReady')}
                    </div>
                  ) : null}

                  {selectedMode === 'CloudCustom' ? (
                    <WizardInput
                      label={t('serverUrl')}
                      value={customServerUrl}
                      onChange={(event) => setCustomServerUrl(event.target.value)}
                      placeholder={t('customServerPlaceholder')}
                      autoCapitalize="none"
                      autoCorrect="off"
                      spellCheck={false}
                      className="font-mono"
                    />
                  ) : null}

                  <button
                    type="button"
                    onClick={handleGoogleAuthenticate}
                    disabled={!selectedBaseUrl || Boolean(savingMode)}
                    className="flex items-center justify-center gap-2 rounded-[1rem] border border-white/10 bg-white/[0.04] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.18em] text-[#F3F3F3] transition-all hover:bg-white/[0.08] disabled:cursor-not-allowed disabled:opacity-45"
                  >
                    {savingMode ? <LoaderCircle size={14} className="animate-spin" /> : null}
                    <span>{t('continueWithGoogle')}</span>
                  </button>

                  <div className="flex items-center gap-3 px-1">
                    <div className="h-px flex-1 bg-white/8" />
                    <span className="text-[9px] font-bold uppercase tracking-[0.22em] text-white/26">{t('or')}</span>
                    <div className="h-px flex-1 bg-white/8" />
                  </div>

                  <WizardInput
                    label={t('email')}
                    value={authMode === 'login' ? loginEmail : email}
                    onChange={(event) =>
                      authMode === 'login'
                        ? setLoginEmail(event.target.value)
                        : setEmail(event.target.value)
                    }
                    placeholder={t('enterEmail')}
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    type="email"
                  />

                  {authMode === 'register' ? (
                    <WizardInput
                      label={t('firstName')}
                      value={firstName}
                      onChange={(event) => setFirstName(event.target.value)}
                      placeholder={t('chooseFirstName')}
                      autoCapitalize="words"
                      autoCorrect="off"
                    />
                  ) : null}

                  {authMode === 'register' ? (
                    <WizardInput
                      label={t('lastName')}
                      value={lastName}
                      onChange={(event) => setLastName(event.target.value)}
                      placeholder={t('optional')}
                      autoCapitalize="words"
                      autoCorrect="off"
                    />
                  ) : null}

                  <WizardInput
                    label={t('password')}
                    type="password"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    placeholder={authMode === 'login' ? t('enterPassword') : t('createPassword')}
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                  />

                  {errorMessage ? (
                    <div className="rounded-[1rem] border border-red-400/18 bg-red-500/8 px-3.5 py-3 text-[12px] leading-5 text-red-100/80">
                      {errorMessage}
                    </div>
                  ) : null}

                  <button
                    type="submit"
                    disabled={!canSubmitRemote || Boolean(savingMode)}
                    className="mt-1 flex items-center justify-center gap-2 rounded-[1rem] bg-[#EFEFEF] px-4 py-3 text-[11px] font-bold uppercase tracking-[0.22em] text-[#080808] transition-all hover:bg-white disabled:cursor-not-allowed disabled:opacity-45"
                  >
                    {savingMode ? <LoaderCircle size={14} className="animate-spin" /> : null}
                    <span>{authMode === 'login' ? t('signInContinue') : t('createAccountContinue')}</span>
                  </button>

                  <p className="text-[10px] leading-5 text-white/28">
                    {selectedMode === 'CloudOfficial'
                      ? t('officialCloudKeychainHint')
                      : t('selfHostedKeychainHint')}
                  </p>
                </form>
              </WizardOptionCard>
            ) : (
              <div className="flex flex-col gap-2">
                {options.map((option) => {
                  const Icon = option.icon;
                  const isSaving = savingMode === option.mode;

                  return (
                    <button
                      key={option.mode}
                      type="button"
                      disabled={Boolean(savingMode)}
                      onClick={() => handleSelectMode(option.mode)}
                      className="group text-left disabled:cursor-wait disabled:opacity-70"
                    >
                      <WizardOptionCard topBorderColor={topBorderColor} grainTheme={CARD_GRAIN_THEME} animatedPalette={sharedAnimation}>
                        <div className="flex items-start gap-3 px-3.5 py-3">
                          <div className="flex h-9 w-9 shrink-0 items-center justify-center text-[#D9DDE4]">
                            <Icon size={17} />
                          </div>
                          <div className="min-w-0 flex-1">
                            <div className="flex items-start justify-between gap-3">
                              <div>
                                <h3 className="text-[16px] font-medium tracking-[-0.02em] text-[#F1F1F1]">{option.title}</h3>
                                <p className="mt-1 pr-1 text-[13px] leading-6 text-white/58">{option.description}</p>
                              </div>
                              <span className="inline-flex shrink-0 items-center pt-0.5 text-[9px] font-bold uppercase tracking-[0.22em] text-white/32 transition-colors duration-200 group-hover:text-white/45">
                                {isSaving ? t('saving') : t('select')}
                              </span>
                            </div>
                          </div>
                        </div>
                      </WizardOptionCard>
                    </button>
                  );
                })}
              </div>
            )}

            <div className="mt-auto px-1 pt-3 text-[9px] uppercase tracking-[0.2em] text-white/22">
              {selectedMode ? t('credentialsChangedLater') : t('changeAnytime')}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
