# Coverage of the IDE Feature Specification

Each item of `IDE Feature Specification.md`, checked against the code as of 2026-11-14 (v0.4.1,
plus B4 run history in progress). Status:

- **Done**: works and is documented in `docs/FEATURES.md`.
- **Partial**: some of it exists; the note says what.
- **Missing**: nothing yet.
- **Network**: needs network access in the app itself, which Coiman doesn't open (it can be
  built only where a tool on the computer does the networking).
- **No AI**: left out by the decision to have no AI features.

**Summary:** 484 items checked, besides the 36 AI items of section 1. Done 77, partial 107,
missing 285, network 14, no AI 1. The "already implemented" table at the end of the
specification is right: every row in it exists.

| Area | Done | Partial | Missing | Network |
|---|---|---|---|---|
| 1 AI (analysis items only) | 4 | 5 | 2 | |
| 2 Architecture | 13 | 4 | 12 | |
| 3 Debugging and performance | 8 | 3 | 27 | |
| 4 Git and collaboration | 1 | 2 | 15 | 9 |
| 5 Build and dependencies | 8 | 15 | 15 | 1 |
| 6 Terminal | 3 | 3 | 12 | |
| 7 Documentation | 1 | 5 | 13 | |
| 8 Web | 1 | 4 | 19 | |
| 9 Database | 5 | 6 | 12 | |
| 10 Data science | 2 | 4 | 16 | |
| 11 Systems and embedded | 1 | 5 | 18 | |
| 12 Game | | 5 | 16 | |
| 13 Mobile | 2 | 4 | 9 | |
| 14 DevOps | | 3 | 19 | 2 |
| 15 Testing | 4 | 5 | 12 | |
| 16 Security and quality | 5 | 5 | 9 | |
| 17 Accessibility and UX | 7 | 6 | 7 | |
| 18 Remote | | | 14 | 1 |
| 19 Plugins | 4 | 4 | 11 | 1 |
| 20 Workspace | 1 | 9 | 7 | |
| 21 Experimental | 7 | 10 | 20 | |

## 1. AI integration and project intelligence

Every AI item (the assistant, chat, code generation, AI editing, models and providers) is
**No AI**. The analysis items work without AI:

| Item | Status | Note |
|---|---|---|
| Automatic project structure analysis | Done | Architecture window |
| Identify architectural patterns | Missing | |
| Detect duplicated code | Done | Code Health, Duplicates |
| Identify potentially unused code | Partial | Files nothing imports (Architecture, Unused); not functions |
| Detect suspicious dependencies | Partial | Code Health licences and audits |
| Explain relationships between classes and modules | Partial | Module graph; type and call hierarchies through LSP |
| Identify potential performance bottlenecks | Missing | |
| Identify security vulnerabilities | Partial | Secrets, `cargo audit`, `npm audit` |
| Analyse project complexity | Done | Code Health, Complexity |
| Generate architecture documentation | Partial | SVG and Mermaid export of the graph |
| Generate diagrams from existing source code | Done | Dependency graph, Mermaid |

## 2. Architecture and dependency analysis

| Item | Status | Note |
|---|---|---|
| Interactive class diagrams | Missing | |
| UML class diagrams | Missing | Mermaid class diagrams preview, but none are generated from code |
| UML sequence diagrams | Missing | Previewed when written in Mermaid |
| UML activity diagrams | Missing | |
| UML component diagrams | Missing | |
| Package dependency diagrams | Done | Architecture, Folders |
| Module dependency graphs | Done | 8 languages |
| Project architecture visualisation | Done | |
| Automatic diagram generation from source code | Done | Dependency graphs |
| Export diagrams to SVG and PNG | Partial | SVG and Mermaid; no PNG |
| Navigate from diagram elements to source code | Done | Double click opens the file |
| Visualise dependencies between modules | Done | |
| Detect circular dependencies | Done | Cycles tab |
| Identify highly coupled modules | Done | Most used, fan-in and fan-out |
| Identify isolated modules | Done | Unused tab |
| Analyse dependency direction | Done | Layered graph |
| Detect architectural violations | Missing | No rules to check against |
| Track dependency changes between commits | Missing | |
| Identify unused dependencies | Missing | Packages declared but not used |
| Show transitive dependency trees | Partial | Maven dependency tree; Cargo crates in the project tree |
| Compare dependency versions | Missing | |
| Detect outdated dependencies | Missing | |
| Files depending on a selected class or function | Partial | Per file (Impact); per symbol through LSP references |
| Identify potential breaking changes | Missing | |
| Show affected tests after a modification | Done | Impact |
| Show affected modules after an API change | Done | Impact |
| Analyse call chains | Done | Call hierarchy (LSP) |
| Visualise execution paths | Missing | |
| Estimate the impact of removing a symbol | Partial | References |

## 3. Advanced debugging and performance analysis

| Item | Status | Note |
|---|---|---|
| Conditional breakpoints with advanced expressions | Done | Conditions and hit counts |
| Logpoints | Done | |
| Data breakpoints | Done | `dataBreakpoint` or gdb's `watch -location` |
| Function breakpoints | Done | |
| Exception breakpoints | Done | Adapter's filters |
| Breakpoint groups | Missing | |
| Breakpoint import and export | Done | |
| Reverse debugging | Missing | |
| Execution recording and replay | Missing | |
| Timeline of executed functions | Missing | |
| Visualisation of call stacks | Done | Call stack view |
| Thread activity visualisation | Partial | Thread list |
| Better multi-threaded debugging | Partial | Threads, pause and step per thread |
| Remote debugging | Missing | Possible through `ssh`/`gdbserver` |
| Attach to running processes | Done | |
| Debug multiple processes simultaneously | Missing | |
| Heap visualisation | Missing | |
| Allocation tracking | Missing | |
| Memory usage over time | Missing | |
| Memory leak detection | Missing | |
| Object lifetime analysis | Missing | |
| Stack memory visualisation | Partial | Memory view (hex) |
| Memory snapshots | Missing | |
| Compare memory snapshots | Missing | |
| Memory ownership analysis | Missing | |
| Reference cycle detection | Missing | |
| Detect excessive allocations | Missing | |
| CPU profiler | Missing | Planned (B5) through `perf`, `samply`, `py-spy` |
| Flame graphs | Missing | B5 |
| Function execution times | Missing | B5 |
| Function call frequency | Missing | B5 |
| Allocation profiling | Missing | |
| Thread performance analysis | Missing | |
| I/O performance analysis | Missing | |
| Performance comparison between executions | Missing | |
| Benchmark history | Missing | |
| Performance regression detection | Missing | |
| Performance changes between commits | Missing | |

## 4. Git and collaboration

| Item | Status | Note |
|---|---|---|
| Interactive rebase interface | Missing | |
| Cherry-pick interface | Missing | |
| Merge conflict resolution editor | Missing | |
| Three-way merge visualisation | Missing | |
| Interactive staging by lines and sections | Done | Line staging |
| Commit graph visualisation | Missing | |
| Branch comparison | Missing | |
| Visualise repository history | Partial | File history, blame |
| Search through commit history | Missing | |
| Compare arbitrary commits | Missing | |
| Git worktree management | Missing | |
| Submodule management | Missing | |
| Git hooks management | Missing | |
| Git LFS integration | Missing | |
| Advanced `.gitignore` editor | Missing | |
| Live collaborative editing | Network | |
| Shared cursors | Network | |
| Shared selections | Network | |
| Follow another developer's cursor | Network | |
| Collaborative debugging | Network | |
| Shared terminals | Network | |
| Shared development sessions | Network | |
| Inline comments on code | Missing | |
| Code review interface | Missing | |
| Review requests and approvals | Network | Possible through `gh` |
| Integration with GitHub and GitLab | Partial | Publish to GitHub through `gh` |
| Conflict detection during collaborative editing | Network | |

## 5. Build systems and dependency management

| Item | Status | Note |
|---|---|---|
| Unified build configuration interface | Partial | Run configurations (`launch.toml`) and the Run picker |
| Visual build task editor | Missing | |
| Build profiles | Partial | Run configurations |
| Debug and release configurations | Partial | Run configurations |
| Custom build pipelines | Missing | |
| Build dependency graphs | Missing | |
| Parallel build monitoring | Missing | |
| Build output filtering | Partial | Output panel, problem matchers |
| Build history | Done | Run History, Builds (B4) |
| Build failure analysis | Partial | First error, problem matchers, Problems |
| Incremental build information | Missing | |
| Build cache management | Missing | |
| Unified dependency manager | Missing | |
| Dependency installation interface | Partial | Maven: search Maven Central and add |
| Dependency removal interface | Missing | |
| Dependency update management | Missing | |
| Dependency vulnerability analysis | Done | Code Health, Audits |
| Dependency licence analysis | Done | Code Health, Licences |
| Dependency conflict detection | Missing | |
| Dependency version comparison | Missing | |
| Lockfile visualisation | Missing | |
| Offline dependency management | Missing | |
| Private package registries | Network | |
| CMake | Done | |
| Make | Done | |
| Ninja | Done | |
| Meson | Partial | Run only |
| Gradle | Partial | Run only |
| Maven | Done | |
| MSBuild | Partial | Its errors are read; no build view |
| Bazel | Missing | |
| Webpack | Partial | Through npm scripts |
| Vite | Partial | npm scripts, dev server address found |
| npm | Done | |
| pnpm | Partial | Run only |
| Yarn | Partial | Run only |
| Poetry | Missing | |
| uv | Partial | Its environments are found |
| Pip | Partial | Environments; debugpy install offer |

## 6. Terminal and command-line environment

| Item | Status | Note |
|---|---|---|
| Multiple terminal sessions | Done | Tabs |
| Terminal profiles | Missing | |
| Custom shell configurations | Partial | Shell choice setting |
| Persistent terminal sessions | Missing | |
| Terminal session restoration | Missing | |
| Split terminal panes | Missing | |
| Terminal tabs | Done | |
| Terminal command history | Missing | The shell's own only |
| Command autocomplete | Missing | The shell's own only |
| Command syntax highlighting | Missing | |
| Command suggestions from project context | Missing | |
| Environment variable editor | Missing | |
| Environment profile management | Missing | |
| SSH terminal integration | Missing | `ssh` runs in a tab, without integration |
| Remote shell support | Missing | |
| Task automation through terminal commands | Partial | Run configurations |
| Terminal output linked to source locations | Partial | Errors go to Problems; `file:line` isn't clickable in the terminal |
| Detect compiler errors in terminal output | Done | Problem matchers |

## 7. Documentation and developer knowledge

| Item | Status | Note |
|---|---|---|
| Documentation browser | Missing | |
| Documentation search | Missing | Hover documentation only |
| Documentation generation | Partial | Doc comment templates |
| API documentation generation | Missing | |
| Automatic documentation updates | Missing | |
| Project documentation index | Missing | |
| Documentation relationship graphs | Missing | |
| Documentation coverage analysis | Missing | |
| Detect outdated documentation | Missing | |
| Detect undocumented public APIs | Missing | |
| Preview documentation in different formats | Partial | Markdown, SVG, Mermaid |
| Markdown documentation workspace | Partial | Preview, outline |
| Mermaid diagram preview | Done | |
| PlantUML integration | Missing | |
| LaTeX preview and editing | Missing | |
| Documentation templates | Partial | Doc comment templates |
| Generate documentation from code comments | Missing | |
| Generate dependency documentation | Missing | |
| Generate architecture reports | Partial | SVG and Mermaid export |

## 8. Web development

| Item | Status | Note |
|---|---|---|
| Integrated browser developer tools | Partial | Page console |
| DOM inspector | Missing | |
| CSS inspector | Missing | |
| Live CSS editing | Partial | Live preview reloads on edit |
| Responsive design preview | Partial | Device sizes in the preview |
| Device viewport presets | Partial | |
| Network request inspector | Missing | |
| Browser console | Done | Page console |
| JavaScript performance profiling | Missing | |
| Accessibility inspection | Missing | |
| Visual component tree | Missing | |
| React, Vue, Angular component inspection | Missing | |
| Hot module replacement management | Missing | |
| HTTP request client | Missing | Possible through `curl` |
| REST API testing environment | Missing | |
| GraphQL explorer | Missing | |
| WebSocket client | Missing | |
| API endpoint explorer | Missing | |
| OpenAPI specification viewer | Missing | |
| API documentation generation | Missing | |
| Request history | Missing | |
| Environment-based API configurations | Missing | |
| Authentication token management | Missing | |
| API response comparison | Missing | |

## 9. Database development

| Item | Status | Note |
|---|---|---|
| Database connection manager | Partial | SQLite files open from the Explorer |
| SQL editor | Done | `.sql` files run from the editor |
| SQL syntax highlighting and completion | Partial | Highlighting; no completion |
| Query execution | Done | |
| Query result viewer | Done | |
| Table data editor | Partial | Through SQL statements |
| Database schema visualisation | Done | Tables, views, columns, keys |
| ER diagrams | Missing | |
| Foreign key visualisation | Partial | Listed in the schema |
| Index inspection | Missing | |
| Query execution plans | Missing | |
| Query performance analysis | Partial | Time per statement |
| Database migration management | Missing | |
| Schema comparison | Missing | |
| Database backup and restore | Missing | |
| Multiple database connections | Partial | One window per database file |
| Connection profiles | Missing | |
| SQLite | Done | Through `sqlite3` |
| PostgreSQL | Missing | Planned through `psql` |
| MySQL and MariaDB | Missing | Planned through `mysql` |
| Microsoft SQL Server | Missing | |
| MongoDB | Missing | |
| Redis browser | Missing | |

## 10. Data science and machine learning

| Item | Status | Note |
|---|---|---|
| Interactive notebook editor | Missing | |
| Jupyter notebook support | Partial | Jupyter kernel sessions (REPL); no `.ipynb` |
| Python notebook execution | Missing | |
| R notebook support | Missing | R REPL |
| Julia notebook support | Missing | Julia REPL |
| Interactive variable explorer | Partial | Debugger variables |
| DataFrame viewer | Missing | |
| Matrix viewer | Missing | |
| Interactive plots | Missing | |
| Plot history | Missing | |
| Dataset preview | Done | CSV and TSV tables |
| CSV data analysis | Partial | Table view, column colours |
| SQL data exploration | Done | SQLite |
| Environment and package management | Partial | Python environments |
| Virtual environment comparison | Missing | |
| GPU monitoring | Missing | |
| CUDA debugging | Missing | |
| Machine learning experiment tracking | Missing | |
| Model visualisation | Missing | |
| Tensor inspection | Missing | |
| Training metrics dashboard | Missing | |
| Model performance comparison | Missing | |

## 11. Systems programming and embedded development

| Item | Status | Note |
|---|---|---|
| Embedded development profiles | Missing | |
| Cross-compilation configuration | Missing | |
| Toolchain manager | Partial | MinGW and build tools found, installs offered |
| Compiler configuration interface | Partial | Compile databases |
| Linker configuration | Missing | |
| Assembly language support | Partial | Disassembly view |
| Assembly-level debugging | Partial | Disassembly while debugging |
| Register visualisation | Partial | Registers in the debugger |
| Peripheral register inspection | Missing | |
| Hardware debugging integration | Missing | |
| GDB integration | Done | |
| OpenOCD integration | Missing | |
| Serial monitor | Missing | |
| UART communication tools | Missing | |
| I2C inspection | Missing | |
| SPI inspection | Missing | |
| Firmware flashing interface | Missing | |
| Microcontroller project templates | Missing | |
| PlatformIO integration | Missing | |
| Arduino development support | Missing | |
| ESP-IDF integration | Missing | |
| Memory map visualisation | Missing | |
| Binary size analysis | Missing | |
| Firmware section analysis | Missing | |

## 12. Game development

| Item | Status | Note |
|---|---|---|
| Game project templates | Missing | |
| Game asset browser | Partial | File Manager and image viewer |
| Sprite viewer | Partial | Image viewer |
| Texture viewer | Partial | Image viewer |
| Animation preview | Partial | Animated GIF and WebP |
| Audio asset preview | Missing | |
| Shader editor | Missing | |
| Shader compilation and diagnostics | Missing | |
| GPU debugging integration | Missing | |
| Scene file inspection | Missing | |
| Game resource dependency analysis | Missing | |
| Physics debugging visualisation | Missing | |
| Game performance profiling | Missing | |
| Frame time analysis | Missing | |
| FPS monitoring | Missing | |
| Engine integration | Missing | |
| Godot integration | Partial | Godot language server |
| Unity project support | Missing | C# editing only |
| Unreal Engine project support | Missing | C++ editing only |
| OpenGL development tools | Missing | |
| Vulkan development tools | Missing | |

## 13. Mobile development

| Item | Status | Note |
|---|---|---|
| Android project support | Partial | Android folders in the project tree |
| Android SDK management | Missing | |
| Android emulator integration | Missing | |
| Android device manager | Missing | |
| ADB terminal integration | Missing | |
| APK inspection | Missing | |
| APK build and signing | Missing | |
| Android logcat viewer | Missing | |
| Gradle Android integration | Missing | |
| Flutter development support | Done | Run, stack traces |
| Dart language tools | Done | |
| iOS project support | Missing | |
| Mobile device preview | Partial | Device sizes in the web preview |
| Responsive layout inspection | Partial | Same |
| Mobile application debugging | Partial | Flutter run |

## 14. DevOps, containers and cloud development

| Item | Status | Note |
|---|---|---|
| Docker integration | Partial | Run rule for a Dockerfile |
| Container management | Missing | |
| Container image inspection | Missing | |
| Dockerfile editor and diagnostics | Partial | Editing and run; no diagnostics |
| Docker Compose management | Partial | Run through `docker compose` |
| Container terminal | Missing | |
| Container debugging | Missing | |
| Kubernetes integration | Missing | |
| Kubernetes resource explorer | Missing | |
| YAML validation for Kubernetes | Missing | YAML tree only |
| Helm chart support | Missing | |
| Terraform integration | Missing | |
| Infrastructure dependency graphs | Missing | |
| Ansible support | Missing | |
| CI/CD pipeline editor | Missing | |
| GitHub Actions integration | Missing | |
| GitLab CI integration | Missing | |
| Pipeline execution history | Missing | |
| Deployment configuration | Missing | |
| Cloud resource explorer | Network | |
| Remote development environments | Missing | |
| SSH project access | Missing | |
| Development containers | Missing | |
| Cloud workspace management | Network | |

## 15. Testing and quality assurance

| Item | Status | Note |
|---|---|---|
| Unified test explorer | Done | Testing view, every language |
| Test execution timeline | Missing | |
| Test history | Done | Run History (B4) |
| Test comparison between runs | Done | Run History: broke, fixed, slower (B4) |
| Flaky test detection | Done | Run History, Flaky (B4) |
| Automatic test reruns | Partial | Run the failed ones again, by hand, for Cargo (B4) |
| Test dependency visualisation | Partial | Impact lists the tests a file reaches |
| Test coverage heatmaps | Partial | Coverage in the gutter |
| Branch coverage | Partial | Read, not shown |
| Mutation testing | Missing | |
| Property-based testing integration | Missing | |
| Fuzz testing integration | Missing | |
| Snapshot testing | Missing | |
| Visual regression testing | Missing | |
| API testing | Missing | |
| Load testing | Missing | |
| Stress testing | Missing | |
| Security testing | Missing | |
| Test environment profiles | Missing | |
| Parallel test execution management | Missing | |
| Test failure analysis | Partial | Failure place, message, Problems, failing streaks |

## 16. Security and code quality

| Item | Status | Note |
|---|---|---|
| Static application security testing | Partial | Likely problems, Clang-Tidy, secrets |
| Dependency vulnerability scanning | Done | Code Health, Audits |
| Secret detection | Done | Code Health, Secrets |
| Hardcoded credential detection | Done | Same |
| Licence compliance | Partial | Licence report; no policy to check against |
| Security-focused code review | Missing | |
| Common vulnerability pattern detection | Missing | |
| SQL injection detection | Missing | |
| Cross-site scripting detection | Missing | |
| Unsafe memory operation detection | Missing | |
| Buffer overflow detection | Missing | |
| Race condition detection | Missing | |
| Security audit reports | Partial | Code Health |
| Code quality dashboards | Partial | Code Health |
| Technical debt tracking | Missing | |
| Complexity analysis | Done | Code Health, Complexity |
| Maintainability analysis | Partial | Complexity, duplicates |
| Code duplication analysis | Done | Code Health, Duplicates |
| Configurable quality gates | Missing | |

## 17. Accessibility and user experience

| Item | Status | Note |
|---|---|---|
| Full screen reader support | Partial | AccessKit is wired in; not checked with a reader |
| Keyboard-only navigation | Partial | Commands and shortcuts for most things |
| Accessibility audit of all components | Missing | |
| High contrast themes | Missing | |
| Custom colour palettes | Done | Themes, VS Code themes |
| Per-language editor layouts | Missing | |
| Per-project interface profiles | Partial | Folder settings |
| Custom panel layouts | Done | Panel bottom, right or in a window; side bar left or right |
| Save and restore workspace layouts | Done | Kept per project; Reset Layout |
| Customisable toolbars | Missing | |
| Command palette extensions | Done | Plugin commands |
| Advanced keybinding editor | Done | |
| Context-sensitive shortcuts | Done | Editor-only and Explorer-only keys |
| Mouse gesture support | Missing | |
| Custom editor rulers | Done | |
| Customisable minimap | Missing | |
| Better font rendering | Partial | |
| Ligature configuration | Missing | |
| Font fallback configuration | Partial | Fallback fonts exist; not configurable |
| Multi-monitor workspace support | Partial | Terminal in its own window |

## 18. Remote development and distributed workflows

| Item | Status | Note |
|---|---|---|
| SSH remote development | Missing | Planned through `ssh` and `sftp` |
| Remote project explorer | Missing | |
| Remote terminal | Missing | |
| Remote LSP execution | Missing | |
| Remote debugging | Missing | |
| Remote build execution | Missing | |
| Remote test execution | Missing | |
| Remote file synchronisation | Missing | |
| Remote workspace profiles | Missing | |
| Remote environment configuration | Missing | |
| Container-based development environments | Missing | |
| Cloud development workspaces | Network | |
| Connection management | Missing | |
| Remote session recovery | Missing | |
| Remote performance monitoring | Missing | |

## 19. Plugin ecosystem

| Item | Status | Note |
|---|---|---|
| Public plugin SDK | Done | `coiman-plugin` |
| Plugin development templates | Missing | |
| Plugin documentation generator | Missing | |
| Plugin API versioning | Done | API v2; older plugins are stopped with the reason |
| Plugin compatibility checker | Done | Same |
| Plugin dependency management | Partial | Lockfile |
| Plugin debugging tools | Missing | |
| Plugin profiling | Missing | |
| Plugin sandbox inspection | Partial | Permissions shown |
| Plugin test framework | Missing | |
| Plugin marketplace interface | Partial | Registries, Extensions view |
| Plugin ratings and reviews | Network | |
| Plugin categories and tags | Missing | |
| Plugin update notifications | Done | Check for plugin updates |
| Plugin changelogs | Missing | |
| Plugin rollback | Missing | |
| Plugin installation profiles | Missing | |
| Workspace-specific plugin configurations | Missing | |
| Plugin discovery through the command palette | Partial | Plugin commands are in the palette; installing isn't |
| Plugin development mode with automatic reload | Missing | |

## 20. Workspace and project management

| Item | Status | Note |
|---|---|---|
| Multi-root workspaces | Missing | |
| Workspace profiles | Partial | Folder settings |
| Project templates | Partial | Sample projects, Maven archetypes, file templates |
| Project scaffolding | Partial | Same |
| Project initialisation wizard | Partial | Sample Project, Git init and clone |
| Project configuration editor | Done | Settings, This folder |
| Workspace-specific environment variables | Missing | |
| Workspace task management | Partial | Run configurations |
| Workspace dependency overview | Partial | Code Health, Licences |
| Workspace health dashboard | Partial | Code Health |
| Project metadata editor | Missing | |
| Project migration tools | Missing | |
| Project backup and restore | Partial | Local history, per file |
| Project snapshots | Missing | |
| Project comparison | Missing | |
| Project-specific keybindings | Missing | |
| Project-specific themes | Partial | Folder settings |
| Project-specific AI instructions | No AI | |

## 21. Experimental and differentiating features

| Item | Status | Note |
|---|---|---|
| Detect the type of project automatically | Done | Run, Testing, language Doctor |
| Identify the tools required by the project | Done | Language Doctor |
| Recommend missing development tools | Done | Install offers |
| Detect incorrect project configurations | Partial | Settings and `launch.toml` problems |
| Identify outdated project conventions | Missing | |
| Suggest improvements to project structure | Missing | |
| Monitor project health | Partial | Code Health |
| Generate a project overview | Missing | |
| Unified project status dashboard | Missing | |
| Execution flow visualisation | Missing | |
| Memory layout visualisation | Missing | |
| Object relationship visualisation | Missing | |
| Data flow diagrams | Missing | |
| Function call graphs | Partial | Call hierarchy |
| Real-time variable visualisation | Missing | |
| Algorithm visualisation | Missing | |
| Interactive recursion visualisation | Missing | |
| Concurrency visualisation | Missing | |
| Structural code editing | Done | |
| Semantic navigation across languages | Partial | LSP, per language |
| Cross-language symbol references | Missing | |
| Intelligent code transformations | Partial | Code actions, refactorings through LSP |
| Visual refactoring | Missing | |
| Interactive AST explorer | Partial | Find by Syntax Tree |
| Query editor for syntax trees | Done | Find by Syntax Tree |
| Custom language grammar support | Partial | Language profiles and registry |
| Custom language server profiles | Done | Server paths, extension servers |
| Personal productivity dashboard | Missing | |
| Coding session statistics stored locally | Missing | |
| Project activity timeline | Partial | Local history timeline, per file |
| Time spent per project | Missing | |
| Task and issue integration | Missing | |
| Personal snippets library | Done | Snippets |
| Reusable project workflows | Partial | Run configurations |
| Automated project setup | Partial | Tool install offers |
| Command sequences | Missing | |
| Custom development routines | Missing | |
