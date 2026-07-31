// Thin wrappers around Tauri invoke calls
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

export { listen };

export const api = {
  loadConfig:      ()              => invoke('load_config'),
  saveConfig:      (projects)      => invoke('save_config', { projects }),
  loadSettings:    ()              => invoke('load_settings'),
  saveSettings:    (settings)      => invoke('save_settings', { settings }),
  getAllStatus:     ()              => invoke('get_all_status'),
  getLogs:         (id, label)     => invoke('get_logs', { id, label }),
  setLogViewer:    (id, label)     => invoke('set_log_viewer', { id, label }),
  startProcess:    (id, command, label, cwd, env) => invoke('start_process', { id, command, label, cwd, env: env || [] }),
  stopProcess:     (id, label)     => invoke('stop_process', { id, label }),
  stopAll:         (id)            => invoke('stop_all_processes', { id }),
  purgeProject:    (id)            => invoke('purge_project', { id }),
  pickFolder:      ()              => invoke('pick_folder'),
  scanProject:     (directory)     => invoke('scan_project', { directory }),
  openInExplorer:  (directory)     => invoke('open_in_explorer', { directory }),
  openInEditor:    (directory, editor) => invoke('open_in_editor', { directory, editor }),
  openInTerminal:  (directory)     => invoke('open_in_terminal', { directory }),
  openInClaude:    (directory, claudeCommand, mode, projectName) => invoke('open_in_claude', { directory, claudeCommand, mode, projectName }),
  openInBrowser:   (url)           => invoke('open_in_browser', { url }),
  forceClose:      ()              => invoke('force_close'),
  getAutostart:    ()              => invoke('get_autostart'),
  setAutostart:    (enabled)       => invoke('set_autostart', { enabled }),
  checkPathsExist: (paths)         => invoke('check_paths_exist', { paths }),
  getDefaultProjectsDir: ()        => invoke('get_default_projects_dir'),
  createProjectFolder: (name)      => invoke('create_project_folder', { name }),
};
