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
    autoMute: false
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

    if (state.switched) {
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
    state.targetDisplayId = elements.targetDisplay.value;

    try {
        await invoke('set_display_settings', {
            data: {
                auto_switch: state.autoSwitch,
                auto_mute: state.autoMute,
                target_id: state.targetDisplayId
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

function bindEvents() {
    elements.btnRestoreDisplay.addEventListener('click', restoreDisplay);
    elements.btnToggleMute.addEventListener('click', toggleMute);
    elements.btnHideToTray.addEventListener('click', hideToTray);
    elements.autoSwitchDisplay.addEventListener('change', saveDisplaySettings);
    elements.autoMuteDisconnect.addEventListener('change', saveDisplaySettings);
    elements.targetDisplay.addEventListener('change', saveDisplaySettings);

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
