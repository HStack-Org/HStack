import { useMemo, useState } from "react";
import { FileText, FolderOpen, Search, TerminalSquare, X, type LucideIcon } from "lucide-react";
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import { SyncProvider } from "./SyncEngine";
import { useSync } from "./useSync";
import { WebGLGrain } from "./components/WebGLGrain";
import { closeAgentWorkspaceWindow, startAgentWorkspaceDrag } from "./platform";

type WorkspaceTabId = "files" | "editor" | "search" | "jobs";

function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const WORKSPACE_IDLE_GRAIN = {
  c1: [26, 26, 26] as [number, number, number],
  c2: [23, 23, 23] as [number, number, number],
  c3: [19, 19, 19] as [number, number, number],
  c4: [16, 16, 16] as [number, number, number],
};

function ContextWindowShell() {
  const { agentWorkspace, agentProgress, agentFilesystemMount, pickAgentFilesystemMount, clearAgentFilesystemMount } = useSync();
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTabId>("files");

  const fileTreeEntries = useMemo(() => Array.isArray(agentWorkspace?.file_tree.entries) ? agentWorkspace.file_tree.entries as Array<{ path?: string; name?: string; file_name?: string; kind?: string }> : [], [agentWorkspace]);
  const fileSearchMatches = useMemo(() => Array.isArray(agentWorkspace?.file_search.matches) ? agentWorkspace.file_search.matches as Array<{ path?: string; line_number?: number; line_text?: string }> : [], [agentWorkspace]);
  const jobHistory = useMemo(() => Array.isArray(agentWorkspace?.jobs.history) ? agentWorkspace.jobs.history as Array<{ summary?: string; state?: string }> : [], [agentWorkspace]);
  const tabs = useMemo(() => ([
    { id: "files", label: "Files", icon: FolderOpen },
    { id: "editor", label: "Editor", icon: FileText },
    { id: "search", label: "Search", icon: Search },
    { id: "jobs", label: "Jobs", icon: TerminalSquare },
  ] as Array<{ id: WorkspaceTabId; label: string; icon: LucideIcon }>), []);

  const handleWindowDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }

    startAgentWorkspaceDrag().catch((error) => console.error("Failed to start workspace window drag:", error));
  };

  const stopWindowChromeInteraction = (event: React.PointerEvent<HTMLElement> | React.MouseEvent<HTMLElement>) => {
    event.stopPropagation();
  };

  return (
    <main className="app-container w-screen h-screen flex flex-col relative bg-[#0B0C0E] rounded-[24px] overflow-hidden border border-white/15 shadow-2xl transition-all duration-300 ease-out" style={{ minHeight: '100dvh' }}>
      <section className="flex-1 min-h-0 flex flex-col relative">
        <div className="absolute inset-0 z-0">
          <WebGLGrain colors={WORKSPACE_IDLE_GRAIN} opacity={0.9} contrast={1.4} />
        </div>
        <div className="absolute top-0 left-0 right-0 h-[1px] bg-white/[0.07] z-10" />
        <div className="relative z-20 flex-1 min-h-0 flex flex-col">
        <div
          onPointerDown={handleWindowDrag}
          className="h-13 min-h-13 border-b border-white/[0.05] bg-white/[0.025] px-2 flex items-end justify-between gap-3 select-none"
        >
          <div className="flex items-end gap-[1px] overflow-x-auto no-scrollbar min-w-0 pt-2 pl-2">
            {tabs.map((tab) => {
              const Icon = tab.icon;
              const active = workspaceTab === tab.id;
              return (
                <button
                  key={tab.id}
                  type="button"
                  onPointerDown={stopWindowChromeInteraction}
                  onClick={() => setWorkspaceTab(tab.id)}
                  className={cn(
                    "appearance-none outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 h-10 px-4 rounded-t-[10px] border border-b-0 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.16em] transition-all whitespace-nowrap relative",
                    active
                      ? "bg-white/[0.04] border-white/[0.06] text-white"
                      : "bg-transparent border-transparent text-white/34 hover:text-white/72 hover:bg-white/[0.03]"
                  )}
                >
                  <Icon size={14} />
                  <span>{tab.label}</span>
                </button>
              );
            })}
          </div>
          <div className="flex items-center gap-2 pb-2.5 pl-2 pr-2 shrink-0">
            {agentProgress ? (
              <div className="hidden md:flex h-8 rounded-full border border-white/[0.05] bg-white/[0.04] px-3 items-center text-[10px] font-semibold uppercase tracking-[0.16em] text-white/42">
                {agentProgress.phase.replace(/_/g, ' ')} · {agentProgress.iteration + 1}
              </div>
            ) : null}
            <button
              type="button"
              onPointerDown={stopWindowChromeInteraction}
              onClick={(event) => {
                stopWindowChromeInteraction(event);
                closeAgentWorkspaceWindow().catch((error) => console.error("Failed to close workspace window:", error));
              }}
              className="appearance-none outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 w-8 h-8 rounded-full flex items-center justify-center text-white/36 hover:text-white hover:bg-white/[0.08] transition-all"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto border-t border-white/[0.03] p-4">
              <div className="mb-3 flex items-center justify-between gap-3 text-[10px] uppercase tracking-[0.16em] text-white/24 px-1">
                <span className="truncate">{agentWorkspace?.dock.focused_app ? `Focused on ${agentWorkspace.dock.focused_app}` : 'Workspace surface'}</span>
                <span className="truncate text-white/18">{agentWorkspace?.file_tree.cwd ?? agentWorkspace?.editor.cwd ?? '/'}</span>
              </div>
              {workspaceTab === "files" ? (
                <div className="rounded-[14px] border border-white/[0.05] bg-white/[0.03] px-3 py-2">
                  <div className="mb-2 flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-[10px] uppercase tracking-[0.18em] text-white/28">{agentWorkspace?.file_tree.cwd ?? "/"}</div>
                      <div className="mt-1 truncate text-[11px] text-white/36">{agentFilesystemMount?.host_path ?? 'No directory mounted'}</div>
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onPointerDown={stopWindowChromeInteraction}
                        onClick={() => void pickAgentFilesystemMount()}
                        className="h-8 rounded-full border border-white/[0.08] bg-white/[0.04] px-3 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.16em] text-white/55 hover:text-white hover:bg-white/[0.08] transition-all"
                      >
                        <FolderOpen size={13} />
                        <span>{agentFilesystemMount?.host_path ? 'Remount' : 'Mount'}</span>
                      </button>
                      {agentFilesystemMount?.host_path ? (
                        <button
                          type="button"
                          onPointerDown={stopWindowChromeInteraction}
                          onClick={() => void clearAgentFilesystemMount()}
                          className="h-8 rounded-full border border-white/[0.08] bg-white/[0.02] px-3 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.16em] text-white/40 hover:text-white/70 hover:bg-white/[0.06] transition-all"
                        >
                          <span>Clear</span>
                        </button>
                      ) : null}
                    </div>
                  </div>
                  <div className="space-y-1.5">
                    {fileTreeEntries.length > 0 ? fileTreeEntries.map((entry, index) => (
                      <div key={`${entry.path ?? entry.name ?? index}`} className="flex items-center justify-between gap-3 text-[12px] text-white/72">
                        <span className="truncate">{entry.name ?? entry.file_name ?? entry.path ?? "entry"}</span>
                        <span className="text-[9px] uppercase tracking-[0.16em] text-white/24">{entry.kind ?? ""}</span>
                      </div>
                    )) : <div className="text-[12px] text-white/35">No filesystem entries loaded.</div>}
                  </div>
                </div>
              ) : null}

              {workspaceTab === "editor" ? (
                <div className="rounded-[14px] border border-white/[0.05] bg-white/[0.03] px-3 py-2">
                  <div className="mb-2 text-[10px] uppercase tracking-[0.18em] text-white/28">{agentWorkspace?.editor.buffer?.path ?? "No file open"}</div>
                  <div className="space-y-0.5 font-mono text-[11px] leading-5 text-white/72">
                    {(agentWorkspace?.editor.buffer?.lines ?? []).slice(0, 80).map((line, index) => (
                      <div key={`${index}-${line}`} className="grid grid-cols-[34px_minmax(0,1fr)] gap-3">
                        <span className="text-right text-white/20">{index + 1}</span>
                        <span className="truncate">{line || " "}</span>
                      </div>
                    ))}
                    {!agentWorkspace?.editor.buffer?.lines?.length ? <div className="font-sans text-[12px] text-white/35">No editor buffer loaded.</div> : null}
                  </div>
                </div>
              ) : null}

              {workspaceTab === "search" ? (
                <div className="rounded-[14px] border border-white/[0.05] bg-white/[0.03] px-3 py-2">
                  <div className="mb-3 flex items-center justify-between gap-4 text-[10px] uppercase tracking-[0.18em] text-white/28">
                    <span>{agentWorkspace?.file_search.focused_query ?? "No search loaded"}</span>
                    <span className="text-white/18">{agentWorkspace?.file_search.scope_root ?? "/"}</span>
                  </div>
                  <div className="space-y-2">
                    {fileSearchMatches.length > 0 ? fileSearchMatches.map((match, index) => (
                      <div key={`${match.path ?? "match"}-${index}`} className="rounded-[12px] border border-white/[0.04] bg-black/20 px-3 py-2">
                        <div className="truncate text-[11px] text-white/70">{match.path ?? "match"}</div>
                        <div className="text-[10px] uppercase tracking-[0.16em] text-white/24">{typeof match.line_number === "number" ? `line ${match.line_number}` : ""}</div>
                        <div className="mt-1 truncate text-[12px] text-white/48">{match.line_text ?? ""}</div>
                      </div>
                    )) : <div className="text-[12px] text-white/35">No search results loaded.</div>}
                  </div>
                </div>
              ) : null}

              {workspaceTab === "jobs" ? (
                <div className="rounded-[14px] border border-white/[0.05] bg-white/[0.03] px-3 py-2">
                  <div className="space-y-2">
                    {jobHistory.length > 0 ? jobHistory.slice().reverse().map((job, index) => (
                      <div key={`${job.summary ?? "job"}-${index}`} className="rounded-[12px] border border-white/[0.04] bg-black/20 px-3 py-2">
                        <div className="text-[12px] text-white/72">{job.summary ?? "job"}</div>
                        <div className="mt-1 text-[10px] uppercase tracking-[0.16em] text-white/24">{job.state ?? "unknown"}</div>
                      </div>
                    )) : <div className="text-[12px] text-white/35">No jobs recorded.</div>}
                  </div>
                </div>
              ) : null}
        </div>
        </div>
      </section>
    </main>
  );
}

export default function ContextWindow() {
  return <SyncProvider userId={1}><ContextWindowShell /></SyncProvider>;
}