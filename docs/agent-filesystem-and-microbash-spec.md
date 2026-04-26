# Agent Filesystem And Microbash Specification

This document defines the intended filesystem, execution, and microbash model for
the HStack agent.

It is intentionally normative.

It is not a brainstorm, not a prompt note, and not an implementation sketch.
It defines the capability boundary that future public and private implementations
must respect.

This specification extends the workspace model in [agent-workspace-viewport-spec.md](agent-workspace-viewport-spec.md)
and must be interpreted consistently with the harness invariants in
[agent-harness-invariants.md](agent-harness-invariants.md) and the public/private
boundary in [public-private-contract.md](public-private-contract.md).

## Status

This document is a target design specification.

- It defines the intended model.
- It does not claim that the current implementation already provides these capabilities.
- If the implementation and this document diverge, this document is the intended design
  unless explicitly superseded.

## Design Goals

The filesystem and microbash model exists to satisfy the following goals:

1. The agent must be able to read and write files inside a bounded workspace.
2. The agent must be unable to escape that workspace by path tricks, links,
   privilege changes, or shell behavior.
3. The agent must be able to create ordinary code and project artifacts when
   those artifacts remain inert until explicit later execution.
4. The agent must be unable to create artifacts that the host OS or ambient host
   environment may execute or activate automatically without an explicit user-triggered
   sandboxed execution step.
5. Microbash must compile to a constrained instruction algebra, not to an OS shell.
6. The filesystem abstraction must support multiple backends with identical user-visible
   semantics, including at minimum a local sandboxed root and future non-local backends
   such as bucket-backed object storage or private remote workspaces.
7. The public implementation must remain real and self-contained without importing
   private orchestration assumptions into shared contracts prematurely.

## Non-Goals

The following are explicitly out of scope for this model:

1. Arbitrary host shell execution.
2. Direct access to host absolute paths.
3. Symlink or hard-link semantics.
4. File ownership, ACL, chmod, xattr, quarantine, or privilege mutation.
5. Transparent access to every host filesystem object type.
6. Background daemons or ambient host-triggered execution.
7. Hidden semantic summarization of filesystem content.

## Threat Model

This specification distinguishes two independent threat classes.

### Threat Class 1: Sandbox Escape

The agent attempts to escape the declared filesystem root or execution boundary by:

- `..` traversal against host paths
- symlink or hard-link traversal
- absolute host paths
- shell expansion
- environment-sensitive path resolution
- device files, FIFOs, sockets, or other special file types
- metadata or privilege mutation

This class is addressed by virtual path semantics, canonicalization, backend validation,
link prohibition, and capability gating.

### Threat Class 2: Ambient Host Activation

The agent writes files that remain inside the sandbox root but are later interpreted,
executed, or activated automatically by the host OS or ambient tooling without an explicit
user-triggered sandboxed execution step.

Examples include:

- OS launch agents or autostart entries
- host profile files in real profile locations
- desktop launcher definitions in real autostart locations
- service definitions or user-session unit files in active host directories

This class is addressed by path-class restrictions and artifact-class restrictions in the
local backend policy.

## Core Principle

The agent does not operate on the host filesystem directly.

The agent operates on a virtual filesystem with explicit semantics.

All tool-visible paths are virtual paths. All normalization occurs in virtual space.
Only canonical virtual paths may be mapped to backend-specific concrete storage locations.

The execution model is separate from the filesystem model.

Microbash is only a user-facing syntax for a constrained instruction language. It is not
a shell.

## Terminology

### Virtual Root

The top-level root of the agent-visible filesystem namespace.

The virtual root is written as `/`.

It is not the host OS root.

### Virtual Path

A path in the agent-visible namespace, interpreted only by the virtual path algebra defined
in this document.

Examples:

- `/src/main.rs`
- `/project/README.md`
- `../notes/today.md` relative to a virtual current working directory

### Canonical Virtual Path

An absolute virtual path with all `.` and `..` segments resolved, no empty interior segments,
and no ability to traverse above `/`.

Examples:

- `/src/lib.rs`
- `/notes/today.md`

Non-examples:

- `src/lib.rs`
- `/src/../lib.rs`
- `/../../etc/passwd`

### Backend

The implementation that stores or retrieves filesystem objects for canonical virtual paths.

Backends may include:

- local sandboxed root backend
- bucket-backed object storage backend
- private remote workspace backend

### Ambient Activation

Any host-side behavior where the mere existence or placement of a file may cause the file,
or a command referenced by the file, to execute or activate without an explicit user-triggered
sandboxed run request.

### Microbash

A constrained user-facing command language which compiles to typed filesystem and execution
instructions.

Microbash is not lowered to `/bin/bash`, `sh`, `cmd.exe`, PowerShell, or any equivalent host
shell.

Microbash has for goal to provide a syntax mostly on par
with ordinary unix/gnu command lines, but far more limited and constrained under the hood avoiding any attack surface of the host machine.

## Virtual Path Model

### Allowed Syntax

The virtual path language supports:

1. `/` as the virtual root
2. `/` as the path separator
3. `.` as the current-directory segment
4. `..` as the parent-directory segment in virtual space only
5. absolute virtual paths beginning with `/`
6. relative virtual paths interpreted against a virtual current working directory

The virtual path language does not support:

1. drive letters
2. backslash separators
3. URI authority sections
4. host home expansion such as `~`
5. shell glob expansion in path parsing
6. environment-variable expansion

### Canonicalization Rules

Let:

- `cwd_v` be the canonical virtual working directory
- `p_user` be the user-provided path
- `canon_v(cwd_v, p_user)` be the canonicalization function

Canonicalization proceeds as follows:

1. If `p_user` is absolute, begin with `/`.
2. If `p_user` is relative, begin with `cwd_v`.
3. Split on `/`.
4. Drop empty segments and `.` segments.
5. For each `..` segment:
   - if the current virtual stack is deeper than `/`, pop one segment
   - otherwise reject with `PathEscapeAboveRoot`
6. Append ordinary segments in order.
7. Reconstruct as an absolute canonical virtual path.

Examples:

- `canon_v(/src, ./main.rs) = /src/main.rs`
- `canon_v(/src/app, ../README.md) = /src/README.md`
- `canon_v(/, ../../etc/passwd)` must reject with `PathEscapeAboveRoot`

### Required Invariant

Backends must never receive unresolved user path strings.

The required order is:

$$
real = map(backend, canonical\_virtual\_path)
$$

The following pattern is forbidden:

$$
canonicalize\_host(base / user\_input)
$$

because it allows backend-specific semantics to influence safety.

## Filesystem Object Model

The virtual filesystem exposes only the following object kinds:

1. regular file
2. directory

The following object kinds are forbidden and must never be created, exposed, or traversed:

1. symbolic link
2. hard link
3. block device
4. character device
5. named pipe (FIFO)
6. socket
7. mount point abstraction
8. reparse point or junction equivalent
9. platform-specific alias, shortcut, or link object that changes resolution semantics

If a backend encounters any forbidden object kind while resolving or operating on a path,
it must fail closed. Any problem need to be raised in the microbash execution context as an explicit error.

## Filesystem Instruction Algebra

The filesystem capability surface is defined as a closed instruction set.

The exact API names may vary, but the semantic set must include only operations equivalent to:

1. `list_dir(path)`
2. `stat(path)`
3. `read_file(path, offset, limit)`
4. `write_file(path, content, mode)`
5. `patch_file(path, patch)`
6. `create_dir(path, recursive)`
7. `move_path(from, to, overwrite)`
8. `delete_path(path, recursive)`
9. `search_text(scope, query, limit)`

The instruction surface must not include:

1. `chmod`
2. `chown`
3. `set_acl`
4. `set_xattr`
5. `set_quarantine`
6. `create_symlink`
7. `create_hardlink`
8. `mount`
9. `unmount`
10. arbitrary host exec

## Backend Contract

Each backend must implement the same semantic contract over canonical virtual paths.

At minimum, a backend must define behavior for:

1. path resolution
2. file reads
3. file writes
4. directory enumeration
5. search
6. metadata retrieval
7. move and delete semantics
8. conflict handling
9. capability denial

### Required Backend Properties

Every backend must be:

1. fail-closed
2. deterministic for canonical paths
3. explicit about unsupported operations
4. incapable of silent path reinterpretation
5. incapable of link traversal

### Local Sandboxed Root Backend

The public baseline backend is a local sandboxed root backend.

Let:

- `R_host` be the configured real host root directory for the sandbox
- `p_v` be a canonical virtual path

Then:

$$
map(local, p_v) = R_{host} / trim\_leading\_slash(p_v)
$$

subject to these requirements:

1. `R_host` must be an HStack-managed root, not an arbitrary host path by default.
2. Resolution must reject any intermediate or leaf object that is a forbidden object kind.
3. The backend must not follow symlinks, junctions, aliases, or equivalent path-redirecting objects.
4. The backend must treat any unsupported or ambiguous host object as an error.

### Bucket-Backed Object Storage Backend

A bucket-backed object storage backend maps canonical virtual paths to object keys or prefixes.

The design must account for this backend class from the outset.

Bucket-backed object storage backends do not support host-special file kinds and therefore inherently
avoid some local-host risks.

The bucket-backed object storage backend must still preserve the same virtual path semantics,
canonicalization rules, capability model, and conflict behavior.

The existence of a bucket-backed backend is not an optional afterthought. The filesystem contract,
instruction algebra, and path model must be designed so that a bucket-backed implementation can be
added later without changing user-visible path semantics or microbash semantics.

### Private Remote Workspace Backend

A private backend may provide stronger isolation or orchestration, such as VM-backed or
container-backed workspaces.

Such capabilities are allowed private extensions, but they must preserve the same virtual path
semantics and explicit capability model.

## Capability Policy Model

Every filesystem operation must run under an explicit policy object.

At minimum, the policy model must define:

1. readable roots
2. writable roots
3. creatable roots
4. deletable roots
5. maximum read size
6. maximum write size
7. maximum directory enumeration size
8. maximum search result count
9. allowed object kinds
10. forbidden path classes
11. forbidden artifact classes

No operation may fall back to a weaker policy automatically.

## Ambient Activation Policy

Ambient activation policy applies only to backends that touch a local host filesystem or any
environment with host-side auto-discovery semantics.

This policy is intentionally narrower than a general "dangerous file" policy.

The mere fact that a file contains code, shell text, build logic, editor configuration, or
task definitions does not make it forbidden.

If a file remains inert until a human explicitly opens it, reads it, or later chooses to run
it in a dedicated sandbox, it is not forbidden by ambient activation policy.

The policy must deny writes to the following path classes and artifact classes.

This list is normative for the public local backend.

### Category A: Host Profile Roots And Shell Initialization Locations

The public local backend must not use any of the following as its sandbox root, nor expose any
subtree mapped onto them:

1. the real user home directory
2. real shell profile directories
3. real OS autostart directories
4. real per-user service directories

If a future backend explicitly maps a virtual subtree onto a real host profile root, the following
effective host paths are forbidden:

1. `.bashrc`
2. `.bash_profile`
3. `.profile`
4. `.zshrc`
5. `.zprofile`
6. `.zlogin`
7. `.zlogout`
8. `.cshrc`
9. `.tcshrc`
10. `.kshrc`
11. `.inputrc`
12. `config.fish`
13. files under `.config/fish/`
14. files under `.config/shell/` if the product later defines such a host-integrated path

### Category B: macOS Ambient Activation Paths

The following path classes are forbidden:

1. `Library/LaunchAgents/*`
2. `Library/LaunchDaemons/*`
3. `System/Library/LaunchAgents/*`
4. `System/Library/LaunchDaemons/*`
5. `Library/StartupItems/*`
6. `Library/LoginItems/*`

The following artifact classes are forbidden anywhere in the local backend if the file is a
recognized macOS launch or application bundle activation artifact:

1. launchd property lists intended for LaunchAgents or LaunchDaemons
2. login-item registration artifacts

### Category C: Linux Ambient Activation Paths

The following path classes are forbidden:

1. `.config/autostart/*`
2. `.config/systemd/user/*`
3. `.local/share/systemd/user/*`
4. `/etc/systemd/system/*` in any backend that could ever map there, which the public local backend must not
5. `.config/upstart/*`
6. `.kde/Autostart/*`
7. `.config/plasma-workspace/env/*`
8. `.config/plasma-workspace/shutdown/*`
9. `/etc/xdg/autostart/*` in any backend that could ever map there, which the public local backend must not

The following artifact classes are forbidden anywhere in the local backend if recognized as
desktop activation artifacts:

1. `.desktop` files intended for autostart or launcher execution
2. user service unit files
3. timer unit files
4. socket unit files

### Category D: Windows Ambient Activation Paths

The following path classes are forbidden:

1. `AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/*`
2. `ProgramData/Microsoft/Windows/Start Menu/Programs/StartUp/*`
3. `Windows/Start Menu/Programs/Startup/*` in any backend that could ever map there, which the public local backend must not
4. `AppData/Roaming/Microsoft/Windows/SendTo/*`

The following artifact classes are forbidden anywhere in the local backend if recognized as host
activation artifacts:

1. `.lnk`
2. `.url`
3. scheduled-task XML intended for registration into the host task scheduler
4. service registration manifests intended for host service installation

### Category E: Cron And Scheduled Execution Surfaces

The following path classes are forbidden when they correspond to real host scheduled-execution
locations:

1. `var/spool/cron/*`
2. `var/at/spool/*`
3. `etc/cron.d/*`
4. `etc/cron.daily/*`
5. `etc/cron.hourly/*`
6. `etc/cron.monthly/*`
7. `etc/cron.weekly/*`
8. user crontab material in any backend that maps to a real host spool location

### Category F: Browser, Desktop, And File-Association Auto-Open Surfaces

The public local backend must forbid artifacts whose ordinary placement would register automatic
open or launch behavior with the host desktop environment.

At minimum, this includes:

1. desktop launcher artifacts in active host launcher directories
2. file-association registration manifests in active host registration directories
3. host login item registration artifacts

### Category G: Direct Execution Metadata And Privilege Surfaces

The following operations are forbidden regardless of path:

1. setting executable bits
2. changing ownership
3. changing group ownership
4. writing ACLs
5. writing extended attributes
6. writing quarantine attributes
7. writing alternate data streams with host execution semantics
8. writing platform-specific privilege manifests intended for host activation

## Explicitly Allowed Inert Artifact Classes

The following file classes are explicitly allowed in `project_sandbox` mode provided they are not
written into any forbidden ambient activation path class:

1. source files in any ordinary language
2. shell scripts
3. Python scripts
4. JavaScript and TypeScript files
5. Rust project files including `Cargo.toml` and `build.rs`
6. Node project files including `package.json`, lockfiles, and ordinary config
7. build files such as `Makefile`, `justfile`, `Taskfile.yml`, `pom.xml`, `build.gradle`, and analogous files
8. editor configuration files such as `.vscode/*`, `.idea/*`, `.zed/*`, and analogous project-local files

These classes are allowed because they remain inert until a human explicitly opens, uses, or runs
them, or until a later dedicated sandbox execution capability is invoked.

Their mere presence in a normal agent-managed workspace root is not treated as ambient activation.

## Capability Profiles

The public implementation must distinguish at least two capability profiles.

### Profile 1: `safe_data_sandbox`

Intended for ordinary note, artifact, and inert document generation.

Allowed:

1. regular file reads and writes
2. directory creation
3. file patching
4. source-code files as inert artifacts
5. scripts as inert artifacts

Forbidden:

1. all Category A through H paths and artifact classes
2. any execution request

### Profile 2: `project_sandbox`

Intended for code and project authoring where files may later be executed explicitly in a separate
sandbox runtime.

Allowed:

1. all operations from `safe_data_sandbox`
2. ordinary source files
3. ordinary project files such as `Cargo.toml`, `package.json`, `Makefile`, `build.rs`, and analogous
   build or task files, provided they remain inert until explicit sandboxed execution

Still forbidden:

1. all Category A through G path classes
2. all Category G operations
3. direct host execution

## Microbash Model

Microbash is a constrained instruction language with a user-facing textual syntax.

It is not a true shell.

### Required Pipeline

The required execution model is:

1. parse microbash into a syntax tree
2. lower the syntax tree into a typed instruction plan
3. validate the plan against filesystem and execution policy
4. execute the plan against the selected backend

The following pipeline is forbidden:

1. render a shell string
2. invoke `/bin/bash`, `/bin/sh`, `cmd.exe`, PowerShell, or equivalent

### Microbash Must Not Support Shell Features With Host Semantics

The public baseline microbash language must not support:

1. command substitution
2. process substitution
3. subshells
4. shell glob expansion with host filesystem semantics
5. variable interpolation from host environment
6. arbitrary redirection semantics
7. pipelines to arbitrary host executables
8. PATH lookup against the host environment
9. sourcing shell scripts
10. shell functions or aliases

### Microbash Lowering Target

Microbash may lower only into:

1. filesystem instructions from the filesystem instruction algebra
2. execution instructions from a separate constrained execution algebra

## Execution Model

Execution is not part of the filesystem abstraction.

Execution must be a separate capability surface with explicit policy.

At minimum, an execution request must include:

1. executable or tool identifier from an allowlist
2. argument vector
3. canonical virtual working directory
4. explicit environment allowlist
5. timeout
6. output size limit
7. filesystem capability profile used during execution

The public baseline implementation must not support arbitrary host executable lookup.

If execution exists publicly, it should target an explicit constrained runtime such as a later
sandbox executor rather than the host shell.

## Workspace Integration

Filesystem capability must integrate with the workspace/app model rather than bypass it.

The intended userland additions are:

1. file tree app
2. editor app
3. filesystem search app
4. execution/job app

These apps must follow the same lifecycle, viewport, and dock rules defined in
[agent-workspace-viewport-spec.md](agent-workspace-viewport-spec.md).

The harness must continue to mount only bounded viewports, not entire filesystems.

## Failure Semantics

All filesystem and microbash failures must be explicit.

Examples include:

1. path escapes above root
2. forbidden object kinds
3. forbidden path classes
4. forbidden artifact classes
5. unsupported operations
6. capability denials
7. size-limit violations
8. policy-profile mismatches

The runtime must not silently coerce a forbidden operation into a weaker allowed operation.

## Validation Requirements

Any implementation of this specification must include tests covering at least:

1. virtual `..` canonicalization within root
2. rejection of traversal above root
3. rejection of symlink traversal
4. rejection of forbidden special file kinds
5. rejection of forbidden ambient activation paths in the local backend
6. allowance of ordinary inert project files in `project_sandbox`
7. proof that microbash is not lowered to a host shell
8. proof that host absolute paths are never accepted as canonical virtual paths

## Public/Private Boundary

The shared contract layer may contain stable protocol and type overlap for virtual paths,
filesystem instructions, and policy enums only if both public and private systems genuinely need
to understand the same concepts.

The following should remain private implementation concerns unless stable overlap emerges:

1. VM orchestration
2. remote workspace management
3. backend credential distribution
4. cloud-specific storage lifecycle
5. privileged execution infrastructure

This follows the boundary in [public-private-contract.md](public-private-contract.md).

## Implementation Sequence

The intended implementation order is:

1. define the virtual path algebra and canonicalization rules in pure code
2. define the filesystem instruction algebra and policy model
3. implement the local sandboxed root backend with Category A through H restrictions
4. integrate filesystem views into the workspace app model
5. add microbash parsing and lowering to the instruction algebra
6. add optional constrained execution as a separate capability
7. extend with private remote or object-storage backends where needed

This order is mandatory because it keeps the safety boundary explicit before convenience surfaces
such as microbash are introduced.

## Testing Requirement

Ideally, all of the semantics in this document should be enforced by automated tests.

In practice, any implementation work derived from this specification should ship with extensive
unit testing from the start.

At minimum, unit tests should cover:

1. virtual path parsing and canonicalization
2. rejection of traversal above virtual root
3. backend path mapping correctness
4. rejection of forbidden object kinds
5. rejection of forbidden ambient activation paths and artifact classes
6. distinction between `safe_data_sandbox` and `project_sandbox`
7. microbash parsing and lowering
8. proof that microbash does not lower to a host shell
9. capability denial behavior
10. local backend conflict and failure semantics
11. bucket-backed backend semantic equivalence for path behavior

If the implementation is not heavily unit-tested, it should be treated as incomplete.
