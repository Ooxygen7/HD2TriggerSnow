const message = document.getElementById("toast-message");
let dismissTimer;

window.__TAURI__.event.listen("show-toast", (event) => {
  message.textContent = String(event.payload?.message || "通知");
  document.body.classList.add("visible");
  clearTimeout(dismissTimer);
  dismissTimer = setTimeout(() => document.body.classList.remove("visible"), 2200);
});

