# Container image and Helm chart

The [container workflow](../.github/workflows/image.yaml) builds the image, runs a smoke test, and publishes to GHCR. It uses `GITHUB_TOKEN` and needs no publishing token. The workflow builds for `linux/amd64`.

A push to `main` that changes code, the Docker build inputs, or the chart is a release. It moves `latest`, and gives the image, the chart and the Git tag the next patch version: `v0.1.0` first, then `v0.1.1`. The image is `ghcr.io/tanguille/8-ball:<version>`. The chart is `oci://ghcr.io/tanguille/charts/eightball`, with the version without the `v`. The GitHub release has generated notes.

A push that changes only docs or this workflow is not a release. For a minor or major version, push the tag `vX.Y.0` on the last commit of `main`, then run `gh workflow run image.yaml --ref main`. That run releases the tag, and the next release continues from it. The same command finishes a release that failed halfway. To build another branch, run `gh workflow run image.yaml --ref <branch>`. That build publishes only the `sha-` image.

The image and chart packages are public, so that deployments and Renovate can read them without credentials. Keep that visibility.

## Helm chart

The [chart](../charts/eightball) uses the bjw-s common library. Its values use the same format as app-template. The image tag defaults to the chart appVersion, which is the release tag. Install it with the release name `eightball`, so the Service is `eightball.<namespace>.svc`:

```sh
helm install eightball oci://ghcr.io/tanguille/charts/eightball --version 0.1.0
```

These options are off by default. Set them to `true` to turn them on:

- `serviceMonitor.app.enabled` scrapes `/metrics`.
- `rawResources.prometheusRule.enabled` adds the `EightballBackendErrors` alert.
- `configMapsFromFolder.enabled` adds the dashboard as the ConfigMap `<release>-dashboard`, with the label `grafana_dashboard: "1"` and the datasource input `DS_PROMETHEUS`.

The chart expects a ConfigMap `<release>-config` with a `config.toml` key, and it creates a 1 GiB PVC for the database. `helm uninstall` keeps that PVC. Set `persistence.data.existingClaim` to use your own PVC.

To use no ConfigMap, set `persistence.config.enabled: false`. Then, in `controllers.eightball.containers.app.env`, set `EIGHTBALL_CONFIG` to `null` and put the TOML in `EIGHTBALL_CONFIG_TOML`.

The image contains the `8ball` CLI and server, not models or server configuration. To run the image:

- Mount a TOML configuration read-only, or set `EIGHTBALL_CONFIG_TOML`.
- Provide writable storage for `EIGHTBALL_DB`.
- Set `EIGHTBALL_API_KEY`.
