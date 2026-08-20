#!/usr/bin/env bash
set -euo pipefail

release_id="${1:?release id is required}"
archive="${2:-/tmp/hd2-stratagem-admin.tar.gz}"
private_key="${3:-/tmp/hd2-catalog-signing-private.pem}"
public_key="${4:-/tmp/hd2-catalog-signing-public.pem}"
app_root=/opt/hd2-stratagem-admin
data_root=/var/lib/hd2-stratagem-admin
release_root="$app_root/releases/$release_id"
nginx_site=/etc/nginx/sites-available/update.unsnow.online
nginx_snippet=/etc/nginx/snippets/update-unsnow-stratagems.conf
backup_root="/var/backups/hd2-stratagem-admin/$release_id"
previous_target=""
nginx_changed=0

rollback() {
  status=$?
  if [[ $status -eq 0 ]]; then
    return
  fi
  if [[ -n "$previous_target" && -d "$previous_target" ]]; then
    ln -sfn "$previous_target" "$app_root/current"
  fi
  if [[ $nginx_changed -eq 1 && -f "$backup_root/update.unsnow.online" ]]; then
    install -m 0644 "$backup_root/update.unsnow.online" "$nginx_site"
    if [[ -f "$backup_root/update-unsnow-stratagems.conf" ]]; then
      install -m 0644 "$backup_root/update-unsnow-stratagems.conf" "$nginx_snippet"
    else
      rm -f "$nginx_snippet"
    fi
    nginx -t && systemctl reload nginx || true
  fi
  systemctl restart hd2-stratagem-admin.service 2>/dev/null || true
  exit "$status"
}
trap rollback EXIT

[[ "$release_id" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]
[[ -f "$archive" ]]

mkdir -p "$backup_root" "$app_root/releases" "$data_root/keys"
install -m 0600 "$nginx_site" "$backup_root/update.unsnow.online"
if [[ -f "$nginx_snippet" ]]; then
  install -m 0600 "$nginx_snippet" "$backup_root/update-unsnow-stratagems.conf"
fi
if [[ -L "$app_root/current" ]]; then
  previous_target="$(readlink -f "$app_root/current")"
fi

if ! id hd2-catalog >/dev/null 2>&1; then
  useradd --system --home-dir "$data_root" --shell /usr/sbin/nologin hd2-catalog
fi

mkdir -p "$release_root"
tar -xzf "$archive" -C "$release_root"
cd "$release_root"
npm ci --omit=dev --no-audit --no-fund
chown -R root:root "$release_root"
chmod -R u=rwX,go=rX "$release_root"

installed_private="$data_root/keys/catalog-signing-private.pem"
installed_public="$data_root/keys/catalog-signing-public.pem"
if [[ ! -f "$installed_private" || ! -f "$installed_public" ]]; then
  [[ -f "$private_key" && -f "$public_key" ]]
  install -m 0600 -o hd2-catalog -g hd2-catalog "$private_key" "$installed_private"
  install -m 0644 -o hd2-catalog -g hd2-catalog "$public_key" "$installed_public"
elif [[ -f "$public_key" ]] && ! cmp -s "$public_key" "$installed_public"; then
  printf 'refusing to replace the existing catalog signing key\n' >&2
  exit 1
fi
chown -R hd2-catalog:hd2-catalog "$data_root"
chmod 0750 "$data_root" "$data_root/keys"

ln -sfn "$release_root" "$app_root/current"
if [[ -n "$previous_target" ]]; then
  ln -sfn "$previous_target" "$app_root/previous"
fi

if [[ ! -f /etc/hd2-stratagem-admin.env ]]; then
  cat >/etc/hd2-stratagem-admin.env <<'ENV'
HD2_CATALOG_HOST=127.0.0.1
HD2_CATALOG_PORT=8785
HD2_CATALOG_DATA=/var/lib/hd2-stratagem-admin
HD2_CATALOG_KEYS=/var/lib/hd2-stratagem-admin/keys
HD2_BUNDLED_ICONS=/opt/hd2-stratagem-admin/current/ui
HD2_SEED_CATALOG=/opt/hd2-stratagem-admin/current/data/seed-catalog.json
HD2_PUBLIC_ORIGIN=https://update.unsnow.online
HD2_ADMIN_DISABLED=1
ENV
  chmod 0600 /etc/hd2-stratagem-admin.env
fi

install -m 0644 "$release_root/deploy/hd2-stratagem-admin.service" /etc/systemd/system/hd2-stratagem-admin.service
install -m 0644 "$release_root/deploy/stratagem-routes.nginx.conf" "$nginx_snippet"
if ! grep -qF 'include /etc/nginx/snippets/update-unsnow-stratagems.conf;' "$nginx_site"; then
  sed -i '/^[[:space:]]*location \/ {/i\    include /etc/nginx/snippets/update-unsnow-stratagems.conf;\n' "$nginx_site"
fi
nginx_changed=1

systemctl daemon-reload
systemctl enable --now hd2-stratagem-admin.service
systemctl restart hd2-stratagem-admin.service
for _ in {1..20}; do
  if curl --fail --silent --show-error http://127.0.0.1:8785/health >/dev/null; then
    break
  fi
  sleep 0.25
done
curl --fail --silent --show-error http://127.0.0.1:8785/api/v1/stratagems/manifest >/dev/null
admin_status="$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:8785/admin/)"
[[ "$admin_status" == "503" ]]

nginx -t
systemctl reload nginx
for _ in {1..20}; do
  if curl --fail --silent --show-error --noproxy '*' \
    --resolve update.unsnow.online:443:127.0.0.1 \
    https://update.unsnow.online/api/v1/stratagems/manifest >/dev/null; then
    break
  fi
  sleep 0.25
done
curl --fail --silent --show-error --noproxy '*' \
  --resolve update.unsnow.online:443:127.0.0.1 \
  https://update.unsnow.online/api/v1/stratagems/manifest >/dev/null

trap - EXIT
printf 'deployed=%s admin_status=%s\n' "$release_id" "$admin_status"
