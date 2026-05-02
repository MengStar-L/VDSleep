const { invoke } = window.__TAURI__.core;

const POLL_INTERVAL_MS = 2000;
const MONITOR_SETTLE_REFRESH_MS = 800;

const state = {
    vdRunning: false,
    connected: false,
    vddInstalled: false,
    monitoring: false,
    baselineReady: false,
    switched: false,
    muted: false,
    monitors: [],
    activeDisplayId: '',
    targetDisplayId: '',
    autoSwitch: true,
    autoMute: false,
    enhancedMode: false,
    restoreKeyScanCode: 0,
    restoreKeyLabel: '',
    autoRestoreOnResume: true,
    restoreWaiting: false,
    recordingKey: false
};

const elements = {};
let pollTimer = null;
let refreshInFlight = false;
let pendingMonitorRefresh = false;
let settleRefreshTimer = null;

function cacheDom() {
    elements.startupOverlay = document.getElementById('startup-overlay');
    elements.startupStatus = document.getElementById('startup-status');
    elements.stepUi = document.getElementById('step-ui');
    elements.stepStatus = document.getElementById('step-status');
    elements.stepMonitors = document.getElementById('step-monitors');
    elements.vdStatusText = document.getElementById('vd-status-text');
    elements.vdIndicator = document.getElementById('vd-indicator');
    elements.vrStatusText = document.getElementById('vr-status-text');
    elements.vrIndicator = document.getElementById('vr-indicator');
    elements.vddStatusText = document.getElementById('vdd-status-text');
    elements.vddIndicator = document.getElementById('vdd-indicator');
    elements.vddWarning = document.getElementById('vdd-warning');
    elements.displayStatusText = document.getElementById('display-status-text');
    elements.displayIndicator = document.getElementById('display-indicator');
    elements.btnRestoreDisplay = document.getElementById('btn-restore-display');
    elements.btnToggleMute = document.getElementById('btn-toggle-mute');
    elements.btnHideToTray = document.getElementById('btn-hide-to-tray');
    elements.autoSwitchDisplay = document.getElementById('auto-switch-display');
    elements.autoMuteDisconnect = document.getElementById('auto-mute-disconnect');
    elements.autoMuteSetting = elements.autoMuteDisconnect.closest('.setting-item');
    elements.enhancedMode = document.getElementById('enhanced-mode');
    elements.restoreKeyRecord = document.getElementById('restore-key-record');
    elements.restoreKeyLabel = document.getElementById('restore-key-label');
    elements.autoRestoreOnResume = document.getElementById('auto-restore-on-resume');
    elements.targetDisplay = document.getElementById('target-display');
    elements.targetWrapper = document.getElementById('target-display-wrapper');
    elements.targetTrigger = document.getElementById('target-display-trigger');
    elements.targetOptions = document.getElementById('target-display-options');
    elements.targetValue = elements.targetTrigger.querySelector('.custom-select-value');
    elements.currentDisplayInfo = document.getElementById('current-display-info');
    elements.baselineInfo = document.getElementById('baseline-info');
}

function setStep(step, status) {
    step.classList.remove('waiting', 'active', 'done');
    step.classList.add(status);
}

function setStartupStatus(text) {
    elements.startupStatus.querySelector('span').textContent = text;
}

function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

function escapeHtml(value) {
    return String(value ?? '').replace(/[&<>"']/g, (char) => ({
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#039;'
    }[char]));
}

function monitorLabel(monitor) {
    const type = monitor.type === 'virtual' ? '虚拟' : '物理';
    const tags = [type];
    if (monitor.primary) tags.push('主显示器');
    if (!monitor.active) tags.push('未启用');
    return `${monitor.name} [${tags.join(' / ')}]`;
}

function hasVirtualDisplayEvidence() {
    return state.vddInstalled || state.monitors.some((monitor) => monitor.type === 'virtual');
}

function updateStatusCards() {
    if (state.vdRunning) {
        elements.vdStatusText.textContent = '已启动';
        elements.vdIndicator.className = 'status-indicator active';
    } else {
        elements.vdStatusText.textContent = '未运行';
        elements.vdIndicator.className = 'status-indicator';
    }

    if (state.connected) {
        elements.vrStatusText.textContent = '已连接';
        elements.vrIndicator.className = 'status-indicator connected';
    } else if (state.vdRunning) {
        elements.vrStatusText.textContent = '等待连接...';
        elements.vrIndicator.className = 'status-indicator warning';
    } else {
        elements.vrStatusText.textContent = '未连接';
        elements.vrIndicator.className = 'status-indicator';
    }

    if (hasVirtualDisplayEvidence()) {
        elements.vddStatusText.textContent = '已安装';
        elements.vddIndicator.className = 'status-indicator active';
        elements.vddWarning.hidden = true;
    } else {
        elements.vddStatusText.textContent = '未安装';
        elements.vddIndicator.className = 'status-indicator';
        elements.vddWarning.hidden = false;
    }

    if (state.restoreWaiting) {
        elements.displayStatusText.textContent = '等待手动按键恢复';
        elements.displayIndicator.className = 'status-indicator warning';
    } else if (state.switched) {
        elements.displayStatusText.textContent = '已切换到目标屏幕';
        elements.displayIndicator.className = 'status-indicator warning';
    } else if (state.baselineReady) {
        elements.displayStatusText.textContent = '原始布局已记录';
        elements.displayIndicator.className = 'status-indicator active';
    } else {
        elements.displayStatusText.textContent = '等待记录原始布局';
        elements.displayIndicator.className = 'status-indicator warning';
    }

    elements.baselineInfo.textContent = state.baselineReady
        ? '已记录，可在断开后恢复'
        : '请保持 VR 未连接，程序会自动记录';
    elements.btnRestoreDisplay.disabled = !state.baselineReady;
}

function updateSettingsUi() {
    elements.autoSwitchDisplay.checked = state.autoSwitch;
    elements.autoMuteDisconnect.checked = state.autoMute;
    const autoMuteText = elements.autoMuteSetting?.querySelector('.setting-text');
    const autoMuteDesc = elements.autoMuteSetting?.querySelector('.setting-desc');
    if (autoMuteText) autoMuteText.textContent = '连接时音量归零';
    if (autoMuteDesc) autoMuteDesc.textContent = '检测到 VR 已连接后，立即将所有播放设备音量设为 0';
    elements.enhancedMode.checked = state.enhancedMode;
    elements.autoRestoreOnResume.checked = state.autoRestoreOnResume;
    elements.restoreKeyLabel.textContent = state.recordingKey
        ? '请按一个键...'
        : (state.restoreKeyLabel || '未设置');
    elements.restoreKeyRecord.classList.toggle('recording', state.recordingKey);
    updateMuteButton();
    updateCurrentDisplay();
    updateCustomSelect();
}

function updateMuteButton() {
    const icon = elements.btnToggleMute.querySelector('.btn-icon');
    const text = elements.btnToggleMute.querySelector('.btn-text');
    icon.textContent = state.muted ? 'ON' : 'AU';
    text.textContent = state.muted ? '取消静音' : '静音';
}

function updateCurrentDisplay() {
    const active =
        state.monitors.find((monitor) => monitor.id === state.activeDisplayId)
        || state.monitors.find((monitor) => monitor.primary)
        || state.monitors.find((monitor) => monitor.active);

    elements.currentDisplayInfo.textContent = active ? active.name : '未检测到显示器';
}

function updateCustomSelect() {
    const selectedValue = state.targetDisplayId || '';
    elements.targetDisplay.innerHTML = '<option value="">自动选择</option>';
    elements.targetOptions.innerHTML = '';

    const addOption = (value, text) => {
        const option = document.createElement('option');
        option.value = value;
        option.textContent = text;
        option.selected = value === selectedValue;
        elements.targetDisplay.appendChild(option);

        const button = document.createElement('button');
        button.type = 'button';
        button.className = `custom-select-option${value === selectedValue ? ' selected' : ''}`;
        button.dataset.value = value;
        button.innerHTML = escapeHtml(text);
        elements.targetOptions.appendChild(button);
    };

    addOption('', '自动选择');
    for (const monitor of state.monitors) {
        addOption(monitor.id, monitorLabel(monitor));
    }

    const target = state.monitors.find((monitor) => monitor.id === selectedValue);
    elements.targetValue.textContent = target ? monitorLabel(target) : '自动选择';
    elements.targetDisplay.value = selectedValue;
}

async function fetchStatus() {
    const data = await invoke('get_status');
    state.vdRunning = data.vd_running || false;
    state.connected = data.connected || false;
    state.vddInstalled = data.vdd_installed || false;
    state.monitoring = data.monitoring || false;
    state.baselineReady = data.baseline_ready || false;
    state.switched = data.switched || false;
    state.restoreWaiting = data.restore_waiting === true;
    updateStatusCards();
}

async function fetchMuteState() {
    const data = await invoke('get_mute_state');
    state.muted = data.muted === true;
    updateMuteButton();
}

async function loadMonitors() {
    const data = await invoke('get_monitors');
    state.monitors = data.monitors || [];
    state.activeDisplayId = data.active_id || '';
    state.targetDisplayId = data.target_id || '';
    state.autoSwitch = data.auto_switch !== false;
    state.autoMute = data.auto_mute === true;
    state.enhancedMode = data.enhanced_mode === true;
    state.restoreKeyScanCode = Number(data.restore_key_scan_code || 0);
    state.restoreKeyLabel = data.restore_key_label || '';
    state.autoRestoreOnResume = data.auto_restore_on_resume !== false;
    state.restoreWaiting = data.restore_waiting === true;
    state.vddInstalled = data.vdd_installed || state.vddInstalled;
    state.baselineReady = data.baseline_ready || state.baselineReady;
    state.switched = data.switched || false;
    updateStatusCards();
    updateSettingsUi();
}

async function refreshAll({ includeMonitors = false } = {}) {
    if (refreshInFlight) {
        pendingMonitorRefresh = pendingMonitorRefresh || includeMonitors;
        return;
    }

    refreshInFlight = true;
    try {
        await fetchStatus();
        if (includeMonitors || state.monitors.length === 0) {
            await loadMonitors();
        }
        await fetchMuteState();
    } catch (error) {
        console.error('[VDSleep] 刷新状态失败:', error);
    } finally {
        refreshInFlight = false;
        if (pendingMonitorRefresh) {
            pendingMonitorRefresh = false;
            void refreshAll({ includeMonitors: true });
        }
    }
}

function scheduleMonitorRefresh(delay = MONITOR_SETTLE_REFRESH_MS) {
    if (settleRefreshTimer) {
        clearTimeout(settleRefreshTimer);
    }

    settleRefreshTimer = setTimeout(() => {
        settleRefreshTimer = null;
        void refreshAll({ includeMonitors: true });
    }, delay);
}

async function saveDisplaySettings() {
    state.autoSwitch = elements.autoSwitchDisplay.checked;
    state.autoMute = elements.autoMuteDisconnect.checked;
    state.enhancedMode = elements.enhancedMode.checked;
    state.autoRestoreOnResume = elements.autoRestoreOnResume.checked;
    state.targetDisplayId = elements.targetDisplay.value;

    try {
        await invoke('set_display_settings', {
            data: {
                auto_switch: state.autoSwitch,
                auto_mute: state.autoMute,
                target_id: state.targetDisplayId,
                enhanced_mode: state.enhancedMode,
                restore_key_scan_code: state.restoreKeyScanCode,
                restore_key_label: state.restoreKeyLabel,
                auto_restore_on_resume: state.autoRestoreOnResume
            }
        });
        await refreshAll({ includeMonitors: true });
        scheduleMonitorRefresh();
    } catch (error) {
        console.error('[VDSleep] 保存显示器设置失败:', error);
    }
}

async function restoreDisplay() {
    elements.btnRestoreDisplay.disabled = true;
    try {
        await invoke('restore_display');
        await refreshAll({ includeMonitors: true });
        scheduleMonitorRefresh();
    } catch (error) {
        console.error('[VDSleep] 恢复屏幕失败:', error);
    } finally {
        elements.btnRestoreDisplay.disabled = !state.baselineReady;
    }
}

async function toggleMute() {
    elements.btnToggleMute.disabled = true;
    try {
        const data = await invoke('toggle_mute');
        if (data.success) {
            state.muted = data.muted === true;
            updateMuteButton();
        }
    } catch (error) {
        console.error('[VDSleep] 切换静音失败:', error);
    } finally {
        elements.btnToggleMute.disabled = false;
    }
}

async function hideToTray() {
    try {
        await invoke('hide_to_tray');
    } catch (error) {
        console.error('[VDSleep] 隐藏到托盘失败:', error);
    }
}

const PHYSICAL_SCAN_CODES = {
    Escape: 0x01,
    Digit1: 0x02,
    Digit2: 0x03,
    Digit3: 0x04,
    Digit4: 0x05,
    Digit5: 0x06,
    Digit6: 0x07,
    Digit7: 0x08,
    Digit8: 0x09,
    Digit9: 0x0A,
    Digit0: 0x0B,
    Minus: 0x0C,
    Equal: 0x0D,
    Backspace: 0x0E,
    Tab: 0x0F,
    KeyQ: 0x10,
    KeyW: 0x11,
    KeyE: 0x12,
    KeyR: 0x13,
    KeyT: 0x14,
    KeyY: 0x15,
    KeyU: 0x16,
    KeyI: 0x17,
    KeyO: 0x18,
    KeyP: 0x19,
    BracketLeft: 0x1A,
    BracketRight: 0x1B,
    Enter: 0x1C,
    ControlLeft: 0x1D,
    KeyA: 0x1E,
    KeyS: 0x1F,
    KeyD: 0x20,
    KeyF: 0x21,
    KeyG: 0x22,
    KeyH: 0x23,
    KeyJ: 0x24,
    KeyK: 0x25,
    KeyL: 0x26,
    Semicolon: 0x27,
    Quote: 0x28,
    Backquote: 0x29,
    ShiftLeft: 0x2A,
    Backslash: 0x2B,
    KeyZ: 0x2C,
    KeyX: 0x2D,
    KeyC: 0x2E,
    KeyV: 0x2F,
    KeyB: 0x30,
    KeyN: 0x31,
    KeyM: 0x32,
    Comma: 0x33,
    Period: 0x34,
    Slash: 0x35,
    ShiftRight: 0x36,
    NumpadMultiply: 0x37,
    AltLeft: 0x38,
    Space: 0x39,
    CapsLock: 0x3A,
    F1: 0x3B,
    F2: 0x3C,
    F3: 0x3D,
    F4: 0x3E,
    F5: 0x3F,
    F6: 0x40,
    F7: 0x41,
    F8: 0x42,
    F9: 0x43,
    F10: 0x44,
    NumLock: 0x45,
    ScrollLock: 0x46,
    Numpad7: 0x47,
    Numpad8: 0x48,
    Numpad9: 0x49,
    NumpadSubtract: 0x4A,
    Numpad4: 0x4B,
    Numpad5: 0x4C,
    Numpad6: 0x4D,
    NumpadAdd: 0x4E,
    Numpad1: 0x4F,
    Numpad2: 0x50,
    Numpad3: 0x51,
    Numpad0: 0x52,
    NumpadDecimal: 0x53,
    F11: 0x57,
    F12: 0x58,
    NumpadEnter: 0xE01C,
    ControlRight: 0xE01D,
    NumpadDivide: 0xE035,
    PrintScreen: 0xE037,
    AltRight: 0xE038,
    Home: 0xE047,
    ArrowUp: 0xE048,
    PageUp: 0xE049,
    ArrowLeft: 0xE04B,
    ArrowRight: 0xE04D,
    End: 0xE04F,
    ArrowDown: 0xE050,
    PageDown: 0xE051,
    Insert: 0xE052,
    Delete: 0xE053,
    MetaLeft: 0xE05B,
    MetaRight: 0xE05C,
    ContextMenu: 0xE05D
};

const KEY_LABELS = {
    Escape: 'Esc',
    Backspace: 'Backspace',
    Tab: 'Tab',
    Enter: 'Enter',
    Space: '空格',
    CapsLock: 'Caps Lock',
    ShiftLeft: '左 Shift',
    ShiftRight: '右 Shift',
    ControlLeft: '左 Ctrl',
    ControlRight: '右 Ctrl',
    AltLeft: '左 Alt',
    AltRight: '右 Alt',
    MetaLeft: '左 Win',
    MetaRight: '右 Win',
    ContextMenu: '菜单键',
    ArrowUp: '方向上',
    ArrowDown: '方向下',
    ArrowLeft: '方向左',
    ArrowRight: '方向右'
};

function keyLabelFromCode(code) {
    if (KEY_LABELS[code]) return KEY_LABELS[code];
    if (code.startsWith('Key')) return code.slice(3);
    if (code.startsWith('Digit')) return code.slice(5);
    if (code.startsWith('Numpad')) return `Numpad ${code.slice(6)}`;
    return code.replace(/([a-z])([A-Z])/g, '$1 $2');
}

function beginKeyRecording() {
    state.recordingKey = true;
    updateSettingsUi();
}

async function recordRestoreKey(event) {
    if (!state.recordingKey) return;

    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;

    const scanCode = PHYSICAL_SCAN_CODES[event.code];
    if (!scanCode) {
        elements.restoreKeyLabel.textContent = '不支持该键';
        setTimeout(updateSettingsUi, 800);
        return;
    }

    state.restoreKeyScanCode = scanCode;
    state.restoreKeyLabel = keyLabelFromCode(event.code);
    state.recordingKey = false;
    updateSettingsUi();
    await saveDisplaySettings();
}

async function restoreFromFocusedShortcut(event) {
    if (state.recordingKey || event.repeat || !state.enhancedMode || !state.restoreKeyScanCode) {
        return;
    }

    const scanCode = PHYSICAL_SCAN_CODES[event.code];
    if (scanCode !== state.restoreKeyScanCode) {
        return;
    }

    event.preventDefault();
    try {
        await invoke('restore_display');
        await refreshAll({ includeMonitors: true });
        scheduleMonitorRefresh();
    } catch (error) {
        console.error('[VDSleep] 快捷键恢复屏幕失败:', error);
    }
}

function bindEvents() {
    elements.btnRestoreDisplay.addEventListener('click', restoreDisplay);
    elements.btnToggleMute.addEventListener('click', toggleMute);
    elements.btnHideToTray.addEventListener('click', hideToTray);
    elements.autoSwitchDisplay.addEventListener('change', saveDisplaySettings);
    elements.autoMuteDisconnect.addEventListener('change', saveDisplaySettings);
    elements.enhancedMode.addEventListener('change', saveDisplaySettings);
    elements.autoRestoreOnResume.addEventListener('change', saveDisplaySettings);
    elements.restoreKeyRecord.addEventListener('click', beginKeyRecording);
    elements.targetDisplay.addEventListener('change', saveDisplaySettings);
    document.addEventListener('keydown', recordRestoreKey, true);
    document.addEventListener('keydown', restoreFromFocusedShortcut, true);

    elements.targetTrigger.addEventListener('click', (event) => {
        event.stopPropagation();
        elements.targetWrapper.classList.toggle('open');
    });

    elements.targetOptions.addEventListener('click', async (event) => {
        const option = event.target.closest('.custom-select-option');
        if (!option) return;

        elements.targetDisplay.value = option.dataset.value || '';
        state.targetDisplayId = elements.targetDisplay.value;
        elements.targetWrapper.classList.remove('open');
        updateCustomSelect();
        await saveDisplaySettings();
    });

    document.addEventListener('click', (event) => {
        if (!elements.targetWrapper.contains(event.target)) {
            elements.targetWrapper.classList.remove('open');
        }
    });

    window.addEventListener('focus', () => {
        void refreshAll({ includeMonitors: true });
    });

    document.addEventListener('visibilitychange', () => {
        if (!document.hidden) {
            void refreshAll({ includeMonitors: true });
        }
    });
}

function startPolling() {
    if (pollTimer) {
        clearTimeout(pollTimer);
    }

    const scheduleNext = () => {
        pollTimer = setTimeout(async () => {
            await refreshAll({ includeMonitors: true });
            scheduleNext();
        }, POLL_INTERVAL_MS);
    };
    scheduleNext();
}

async function init() {
    cacheDom();
    bindEvents();

    setStep(elements.stepUi, 'active');
    setStartupStatus('正在初始化界面组件...');
    await sleep(120);
    setStep(elements.stepUi, 'done');

    setStep(elements.stepStatus, 'active');
    setStartupStatus('正在检测 VD Streamer 与系统状态...');
    await fetchStatus();
    await fetchMuteState();
    setStep(elements.stepStatus, 'done');

    setStep(elements.stepMonitors, 'active');
    setStartupStatus('正在检测显示器列表...');
    await loadMonitors();
    setStep(elements.stepMonitors, 'done');

    setStartupStatus('启动完成');
    await sleep(250);
    elements.startupOverlay.classList.add('hidden');
    startPolling();
}

document.addEventListener('DOMContentLoaded', () => {
    init().catch(async (error) => {
        console.error('[VDSleep] 初始化失败:', error);
        setStartupStatus(`初始化失败：${error.message || error}`);
        await sleep(1500);
        elements.startupOverlay.classList.add('hidden');
    });
});
