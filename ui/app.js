const tauri = window.__TAURI__;
const invoke = tauri.core.invoke;

function byId(id) {
  return document.getElementById(id);
}

async function renderMigrationStatus() {
  const report = await invoke("migration_status");
  const title = byId("migration-title");
  const detail = byId("migration-detail");

  if (!report.legacyDirectoryFound) {
    title.textContent = "未发现旧版配置";
    detail.textContent = "新版将以独立数据目录运行。";
    return;
  }

  if (report.importedFiles.length > 0) {
    title.textContent = `已导入 ${report.importedFiles.length} 份旧版配置`;
    detail.textContent = report.importedFiles.join(" · ");
    return;
  }

  title.textContent = "旧版配置已保留";
  detail.textContent = "迁移标记已存在；新版不会再次覆盖自己的数据。";
}

async function bootstrap() {
  byId("version").textContent = `v${await invoke("get_app_version")}`;
  await renderMigrationStatus();

  byId("toggle-overlay").addEventListener("click", async () => {
    const visible = await invoke("toggle_overlay");
    await invoke("show_toast", { payload: { message: visible ? "OVERLAY ON" : "OVERLAY OFF" } });
  });

  byId("show-toast").addEventListener("click", () =>
    invoke("show_toast", { payload: { message: "RUST EVENT CHANNEL ONLINE" } })
  );
}

bootstrap().catch((error) => {
  byId("migration-title").textContent = "启动检查失败";
  byId("migration-detail").textContent = String(error);
});

