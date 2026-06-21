const state = document.getElementById("overlay-state");

window.__TAURI__.event.listen("render-overlay", (event) => {
  const loadout = Array.isArray(event.payload) ? event.payload : [];
  state.textContent = loadout.length > 0 ? `${loadout.length} 个战备已同步` : "等待配装数据";
});

