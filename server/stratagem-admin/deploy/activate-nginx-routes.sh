#!/usr/bin/env bash
set -euo pipefail

release_id="${1:?release id is required}"
site=/etc/nginx/sites-available/update.unsnow.online
snippet=/etc/nginx/snippets/update-unsnow-stratagems.conf
source_snippet=/opt/hd2-stratagem-admin/current/deploy/stratagem-routes.nginx.conf
backup="/var/backups/hd2-stratagem-admin/$release_id-nginx"
mkdir -p "$backup"
install -m 0600 "$site" "$backup/update.unsnow.online"
if [[ -f "$snippet" ]]; then
  install -m 0600 "$snippet" "$backup/update-unsnow-stratagems.conf"
fi

rollback() {
  status=$?
  if [[ $status -eq 0 ]]; then
    return
  fi
  install -m 0644 "$backup/update.unsnow.online" "$site"
  if [[ -f "$backup/update-unsnow-stratagems.conf" ]]; then
    install -m 0644 "$backup/update-unsnow-stratagems.conf" "$snippet"
  else
    rm -f "$snippet"
  fi
  nginx -t && systemctl reload nginx || true
  exit "$status"
}
trap rollback EXIT

install -m 0644 "$source_snippet" "$snippet"
if ! grep -qF 'include /etc/nginx/snippets/update-unsnow-stratagems.conf;' "$site"; then
  sed -i '/^[[:space:]]*location \/ {/i\    include /etc/nginx/snippets/update-unsnow-stratagems.conf;\n' "$site"
fi
nginx -t
systemctl reload nginx
manifest_status=""
for _ in {1..20}; do
  manifest_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
    --noproxy '*' \
    --resolve update.unsnow.online:443:127.0.0.1 \
    https://update.unsnow.online/api/v1/stratagems/manifest)"
  [[ "$manifest_status" == "200" ]] && break
  sleep 0.25
done
if [[ "$manifest_status" != "200" ]]; then
  grep -n -B 2 -A 2 'stratagem' "$site" || true
  cat "$snippet" || true
  curl --silent --show-error --include \
    --noproxy '*' \
    --resolve update.unsnow.online:443:127.0.0.1 \
    https://update.unsnow.online/api/v1/stratagems/manifest || true
  exit 1
fi
admin_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --noproxy '*' \
  --resolve update.unsnow.online:443:127.0.0.1 \
  https://update.unsnow.online/admin/)"
[[ "$admin_status" == "503" ]]

trap - EXIT
printf 'nginx_routes=active admin_status=%s\n' "$admin_status"
