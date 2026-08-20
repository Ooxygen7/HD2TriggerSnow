"use strict";

const GROUP_NAMES = Object.freeze({
  support: "支援武器",
  orbital: "轨道",
  eagle: "飞鹰",
  emplacement: "防御工事",
  sentry: "哨戒炮",
  backpack: "背包",
  vehicle: "载具",
  mission: "任务",
});

const DIRECTION_SYMBOLS = Object.freeze({ W: "↑", A: "←", S: "↓", D: "→" });
const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$/;

const elements = Object.fromEntries([
  "catalog-version", "session-user", "history-button", "search-input", "add-button",
  "catalog-count", "catalog-list", "stratagem-form", "editor-category", "editor-title",
  "enabled-input", "id-input", "name-zh-input", "name-en-input", "group-input",
  "ocr-input", "aliases-input", "sequence-input", "clear-sequence", "icon-button",
  "icon-filename", "icon-input", "revert-button", "save-button", "preview-icon",
  "preview-name", "preview-sequence", "publish-dialog", "publish-summary", "publish-confirm",
  "history-dialog", "history-close", "history-list", "rollback-dialog", "rollback-summary",
  "rollback-confirm", "toast",
].map((id) => [id, document.getElementById(id)]));

const state = {
  manifest: null,
  items: [],
  selectedId: null,
  baseline: null,
  draftIcon: null,
  creating: false,
  publishing: false,
  rollbackVersion: null,
  toastTimer: null,
};

function clone(value) {
  return structuredClone(value);
}

function iconUrl(icon) {
  if (!icon) return "";
  if (icon.kind === "bundled") {
    return `api/bundled-icon/${encodeURIComponent(icon.value)}`;
  }
  return `data:${icon.mediaType};base64,${icon.base64}`;
}

function splitTerms(value) {
  const seen = new Set();
  return String(value || "")
    .split(/[,，]/)
    .map((term) => term.trim())
    .filter((term) => {
      const key = term.toLocaleLowerCase();
      if (!term || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

function currentItem() {
  return state.items.find((item) => item.id === state.selectedId) || null;
}

function apiErrorMessage(error) {
  if (error?.code === "version_conflict") return "数据库已被更新，已重新载入";
  if (error?.status === 403) return "登录已失效，请刷新页面";
  if (error?.status === 413) return "文件或数据过大";
  if (error?.status >= 500) return "服务器暂时无法完成操作";
  return error?.message || "操作失败";
}

async function api(path, options = {}) {
  const request = { credentials: "same-origin", ...options };
  request.headers = new Headers(options.headers || {});
  if (request.body != null) {
    request.headers.set("Content-Type", "application/json");
    request.headers.set("X-HD2-Admin", "1");
  }
  const response = await fetch(`api/${path}`, request);
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    const error = new Error(payload.message || payload.error || `HTTP ${response.status}`);
    error.status = response.status;
    error.code = payload.error;
    error.currentVersion = payload.currentVersion;
    throw error;
  }
  return payload;
}

function showToast(message, tone = "neutral") {
  clearTimeout(state.toastTimer);
  elements.toast.textContent = message;
  elements.toast.dataset.tone = tone;
  elements.toast.classList.add("visible");
  state.toastTimer = setTimeout(() => elements.toast.classList.remove("visible"), 2600);
}

function setEditorEnabled(enabled) {
  for (const control of elements["stratagem-form"].querySelectorAll("input, select, button")) {
    control.disabled = !enabled;
  }
  elements["id-input"].readOnly = enabled && !state.creating;
}

function draftFromForm() {
  return {
    id: elements["id-input"].value.trim(),
    grp: elements["group-input"].value,
    name: {
      zh: elements["name-zh-input"].value.trim(),
      en: elements["name-en-input"].value.trim(),
    },
    aliases: splitTerms(elements["aliases-input"].value),
    ocr: splitTerms(elements["ocr-input"].value),
    seq: elements["sequence-input"].value.trim().toUpperCase().split(""),
    icon: state.draftIcon ? clone(state.draftIcon) : null,
    enabled: elements["enabled-input"].checked,
    order: state.baseline?.order ?? state.items.length,
  };
}

function isDirty() {
  if (!state.baseline) return false;
  return JSON.stringify(draftFromForm()) !== JSON.stringify(state.baseline);
}

function updateDirtyState() {
  const dirty = isDirty();
  elements["save-button"].disabled = !dirty || state.publishing;
  elements["revert-button"].disabled = !dirty || state.publishing;
  updatePreview();
}

function updatePreview() {
  if (!state.baseline) {
    elements["preview-name"].textContent = "—";
    elements["preview-sequence"].textContent = "—";
    elements["preview-icon"].removeAttribute("src");
    return;
  }
  const draft = draftFromForm();
  elements["preview-name"].textContent = draft.name.zh || draft.name.en || "—";
  elements["preview-sequence"].textContent = draft.seq.map((key) => DIRECTION_SYMBOLS[key] || key).join(" ") || "—";
  const source = iconUrl(draft.icon);
  if (source) {
    elements["preview-icon"].src = source;
    elements["preview-icon"].alt = draft.name.zh ? `${draft.name.zh}图标` : "战备图标";
  } else {
    elements["preview-icon"].removeAttribute("src");
    elements["preview-icon"].alt = "";
  }
  document.getElementById("client-card").classList.toggle("disabled", !draft.enabled);
}

function fillEditor(item, creating = false) {
  state.creating = creating;
  state.selectedId = creating ? null : item.id;
  state.baseline = clone(item);
  state.draftIcon = clone(item.icon);
  elements["id-input"].value = item.id;
  elements["name-zh-input"].value = item.name.zh;
  elements["name-en-input"].value = item.name.en;
  elements["group-input"].value = item.grp;
  elements["ocr-input"].value = item.ocr.join("，");
  elements["aliases-input"].value = item.aliases.join("，");
  elements["sequence-input"].value = item.seq.join("");
  elements["enabled-input"].checked = item.enabled;
  elements["icon-filename"].textContent = item.icon?.kind === "bundled" ? item.icon.value : "已规范化图标";
  elements["editor-category"].textContent = creating ? "新建" : GROUP_NAMES[item.grp];
  elements["editor-title"].textContent = creating ? "新战备" : item.name.zh;
  setEditorEnabled(true);
  updateDirtyState();
  renderList();
}

function selectItem(id) {
  if (isDirty() && !window.confirm("放弃未保存的更改？")) return;
  const item = state.items.find((entry) => entry.id === id);
  if (item) fillEditor(item);
}

function renderList() {
  const query = elements["search-input"].value.trim().toLocaleLowerCase();
  const filtered = state.items.filter((item) => {
    const haystack = [item.id, item.name.zh, item.name.en, ...item.aliases, ...item.ocr].join("\n").toLocaleLowerCase();
    return !query || haystack.includes(query);
  });
  const fragment = document.createDocumentFragment();
  for (const item of filtered) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "catalog-item";
    if (!state.creating && item.id === state.selectedId) button.setAttribute("aria-current", "true");
    button.classList.toggle("disabled", !item.enabled);
    button.addEventListener("click", () => selectItem(item.id));

    const icon = document.createElement("img");
    icon.src = iconUrl(item.icon);
    icon.alt = "";
    const copy = document.createElement("span");
    copy.className = "catalog-item-copy";
    const name = document.createElement("strong");
    name.textContent = item.name.zh;
    const meta = document.createElement("span");
    meta.textContent = `${GROUP_NAMES[item.grp]} · ${item.seq.map((key) => DIRECTION_SYMBOLS[key]).join("")}`;
    copy.append(name, meta);
    button.append(icon, copy);
    if (!item.enabled) {
      const badge = document.createElement("span");
      badge.className = "disabled-badge";
      badge.textContent = "停用";
      button.append(badge);
    }
    fragment.append(button);
  }
  elements["catalog-list"].replaceChildren(fragment);
  elements["catalog-count"].textContent = `${filtered.length} 项`;
}

function validateDraft(draft) {
  const form = elements["stratagem-form"];
  for (const input of form.querySelectorAll("input")) input.setCustomValidity("");
  if (!ID_PATTERN.test(draft.id) || draft.id.toLowerCase().startsWith("custom_")) {
    elements["id-input"].setCustomValidity("仅可使用字母、数字、点、下划线和连字符");
  } else if (state.creating && state.items.some((item) => item.id.toLocaleLowerCase() === draft.id.toLocaleLowerCase())) {
    elements["id-input"].setCustomValidity("标识符已存在");
  }
  if (!draft.name.zh) elements["name-zh-input"].setCustomValidity("请输入中文名");
  if (!draft.name.en) elements["name-en-input"].setCustomValidity("请输入英文名");
  if (!draft.seq.length || !draft.seq.every((key) => Object.hasOwn(DIRECTION_SYMBOLS, key))) {
    elements["sequence-input"].setCustomValidity("仅可输入 W、A、S、D");
  }
  if (!draft.icon) {
    showToast("请选择图标", "error");
    return false;
  }
  return form.reportValidity();
}

function publishSummary(draft) {
  if (state.creating) return `新增 ${draft.name.zh}`;
  if (draft.enabled !== state.baseline.enabled) return `${draft.enabled ? "恢复" : "停用"} ${draft.name.zh}`;
  return `更新 ${draft.name.zh}`;
}

async function publishDraft() {
  if (state.publishing) return;
  const draft = draftFromForm();
  if (!validateDraft(draft)) return;
  state.publishing = true;
  updateDirtyState();
  elements["publish-confirm"].disabled = true;
  try {
    const items = state.creating
      ? [...state.items.map(clone), draft]
      : state.items.map((item) => item.id === state.selectedId ? draft : clone(item));
    const result = await api("catalog", {
      method: "PUT",
      body: JSON.stringify({ baseVersion: state.manifest.catalogVersion, items }),
    });
    state.manifest = result.manifest;
    state.items = result.catalog.items;
    elements["catalog-version"].textContent = `v${state.manifest.catalogVersion}`;
    const saved = state.items.find((item) => item.id === draft.id);
    fillEditor(saved);
    showToast("已发布", "success");
  } catch (error) {
    if (error.code === "version_conflict") await loadCatalog();
    showToast(apiErrorMessage(error), "error");
  } finally {
    state.publishing = false;
    elements["publish-confirm"].disabled = false;
    updateDirtyState();
  }
}

async function loadCatalog(preferredId = state.selectedId) {
  const result = await api("catalog");
  state.manifest = result.manifest;
  state.items = result.catalog.items;
  elements["catalog-version"].textContent = `v${state.manifest.catalogVersion}`;
  renderList();
  const item = state.items.find((entry) => entry.id === preferredId) || state.items[0];
  if (item) fillEditor(item);
}

async function loadHistory() {
  elements["history-list"].replaceChildren();
  try {
    const { history } = await api("history?limit=100");
    const fragment = document.createDocumentFragment();
    for (const entry of history) {
      const row = document.createElement("div");
      row.className = "history-row";
      const copy = document.createElement("div");
      copy.className = "history-meta";
      const title = document.createElement("strong");
      title.textContent = `v${entry.version}`;
      const meta = document.createElement("span");
      const action = entry.action === "rollback" ? `回滚自 v${entry.sourceVersion}` : entry.action === "initialize" ? "初始化" : "发布";
      meta.textContent = `${action} · ${new Date(entry.publishedAt).toLocaleString("zh-CN", { hour12: false })}`;
      copy.append(title, meta);
      row.append(copy);
      if (entry.version !== state.manifest.catalogVersion) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "button secondary compact";
        button.textContent = "回滚";
        button.addEventListener("click", () => {
          state.rollbackVersion = entry.version;
          elements["rollback-summary"].textContent = `将 v${entry.version} 的内容发布为新版本。`;
          elements["history-dialog"].close();
          elements["rollback-dialog"].showModal();
        });
        row.append(button);
      }
      fragment.append(row);
    }
    elements["history-list"].replaceChildren(fragment);
  } catch (error) {
    showToast(apiErrorMessage(error), "error");
  }
}

async function rollback() {
  const targetVersion = state.rollbackVersion;
  if (!targetVersion || state.publishing) return;
  state.publishing = true;
  elements["rollback-confirm"].disabled = true;
  try {
    const result = await api("rollback", {
      method: "POST",
      body: JSON.stringify({ baseVersion: state.manifest.catalogVersion, targetVersion }),
    });
    state.manifest = result.manifest;
    state.items = result.catalog.items;
    elements["catalog-version"].textContent = `v${state.manifest.catalogVersion}`;
    renderList();
    fillEditor(state.items[0]);
    showToast(`已回滚并发布 v${state.manifest.catalogVersion}`, "success");
  } catch (error) {
    if (error.code === "version_conflict") await loadCatalog();
    showToast(apiErrorMessage(error), "error");
  } finally {
    state.publishing = false;
    elements["rollback-confirm"].disabled = false;
    state.rollbackVersion = null;
  }
}

async function normalizeIcon(file) {
  if (!file) return;
  if (file.size > 1024 * 1024) return showToast("图标不能超过 1 MiB", "error");
  const allowed = new Set(["image/svg+xml", "image/png", "image/jpeg"]);
  if (!allowed.has(file.type)) return showToast("仅支持 SVG、PNG 或 JPEG", "error");
  elements["icon-button"].disabled = true;
  try {
    const dataUrl = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error || new Error("读取文件失败"));
      reader.readAsDataURL(file);
    });
    const base64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
    const { icon } = await api("icons/normalize", {
      method: "POST",
      body: JSON.stringify({ mediaType: file.type, base64 }),
    });
    state.draftIcon = icon;
    elements["icon-filename"].textContent = file.name;
    updateDirtyState();
  } catch (error) {
    showToast(apiErrorMessage(error), "error");
  } finally {
    elements["icon-button"].disabled = false;
    elements["icon-input"].value = "";
  }
}

elements["search-input"].addEventListener("input", renderList);
elements["add-button"].addEventListener("click", () => {
  if (isDirty() && !window.confirm("放弃未保存的更改？")) return;
  fillEditor({
    id: "", grp: "support", name: { zh: "", en: "" }, aliases: [], ocr: [], seq: [],
    icon: null, enabled: true, order: Math.max(-1, ...state.items.map((item) => item.order)) + 1,
  }, true);
  elements["id-input"].focus();
});
elements["stratagem-form"].addEventListener("input", () => {
  elements["sequence-input"].value = elements["sequence-input"].value.toUpperCase().replace(/[^WASD]/g, "");
  elements["editor-category"].textContent = state.creating ? "新建" : GROUP_NAMES[elements["group-input"].value];
  elements["editor-title"].textContent = elements["name-zh-input"].value.trim() || (state.creating ? "新战备" : "未命名");
  updateDirtyState();
});
elements["stratagem-form"].addEventListener("submit", (event) => {
  event.preventDefault();
  const draft = draftFromForm();
  if (!validateDraft(draft)) return;
  elements["publish-summary"].textContent = `${publishSummary(draft)}，客户端将在下次启动时收到。`;
  elements["publish-dialog"].showModal();
});
elements["publish-dialog"].addEventListener("close", () => {
  if (elements["publish-dialog"].returnValue === "default") void publishDraft();
});
elements["revert-button"].addEventListener("click", () => fillEditor(state.baseline, state.creating));
elements["icon-button"].addEventListener("click", () => elements["icon-input"].click());
elements["icon-input"].addEventListener("change", () => void normalizeIcon(elements["icon-input"].files[0]));
elements["clear-sequence"].addEventListener("click", () => {
  elements["sequence-input"].value = "";
  updateDirtyState();
  elements["sequence-input"].focus();
});
for (const button of document.querySelectorAll("[data-direction]")) {
  button.addEventListener("click", () => {
    if (elements["sequence-input"].value.length < 32) elements["sequence-input"].value += button.dataset.direction;
    updateDirtyState();
  });
}
elements["history-button"].addEventListener("click", () => {
  void loadHistory();
  elements["history-dialog"].showModal();
});
elements["history-close"].addEventListener("click", () => elements["history-dialog"].close());
elements["rollback-dialog"].addEventListener("close", () => {
  if (elements["rollback-dialog"].returnValue === "default") void rollback();
});
window.addEventListener("beforeunload", (event) => {
  if (isDirty()) event.preventDefault();
});

async function initialize() {
  try {
    const session = await api("session");
    elements["session-user"].textContent = session.user;
    await loadCatalog();
    document.querySelector("main").setAttribute("aria-busy", "false");
  } catch (error) {
    showToast(apiErrorMessage(error), "error");
  }
}

void initialize();
