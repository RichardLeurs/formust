// Facts taken from README.md and docs/specs.md (keep in step with them).

export const features = [
  {
    title: 'Free Pascal inside',
    text: 'The compilers ship with the app: Apple Silicon, Intel and Windows 64-bit, built from the published source. Only the Command Line Tools are needed.',
  },
  {
    title: 'Native form designer',
    text: 'Real AppKit controls on the design surface, a palette of 115 components, menus, action lists, anchors, and Lazarus .lfm files.',
  },
  {
    title: 'Coding agents',
    text: 'Claude Console API, Claude Code, OpenAI API, Codex CLI and OpenAI-compatible servers such as oMLX, Ollama and LM Studio. Every change is a diff you apply or reject.',
  },
  {
    title: 'Export anywhere',
    text: 'One click makes an .app and .dmg for Apple Silicon, Intel or universal, or an .exe and .zip for Windows.',
  },
]

export const editor = [
  'Pascal highlighting, tabs, file tree, errors and warnings underlined as you type',
  'Completion as you type, hover, go-to-definition (F12) and Format Code, through pasls',
  'Gutter breakpoints, step, call stack and a variables pane through lldb-dap',
  'Integrated terminal with colours, scrollback and full-screen programs such as vim',
  'Project files: .lpi, .dproj, .lpk and .dpk are read; Debug and Release builds',
  'Light and dark appearance, native menus, Preferences for fonts and colours',
]

export const status = [
  { area: 'Debugger', state: 'working', notes: 'breakpoints, step, call stack, variables; Pascal sets are not shown' },
  { area: 'Code completion', state: 'working', notes: 'completion, hover, go-to-definition, parameter hints, rename' },
  { area: 'Build and run', state: 'working', notes: 'console and GUI programs, Problems pane, export to .app and .dmg' },
  { area: 'Form designer', state: 'working', notes: 'native controls, menus, action lists, anchors, frames, data modules' },
  { area: 'AI assistant', state: 'working', notes: 'Claude, OpenAI, Codex CLI, Claude Code and local servers; keys in the Keychain' },
  { area: 'Packaging', state: 'partly', notes: 'three apps and disk images are built; none run on an Intel Mac; notarization not run' },
]

export const requirements = [
  'An Apple Silicon Mac with macOS 14 Sonoma or later (the IDE was developed on macOS 27)',
  'Xcode or the Command Line Tools (the linker and the macOS SDK; lldb-dap ships in Xcode)',
  'Rust, stable, only to build from source',
]

export const buildSteps = [
  'cargo run -p pascoda-app -- tests/fixtures/debug.pas',
  'cargo test',
]
