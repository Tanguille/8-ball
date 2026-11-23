# Container image and Helm chart

## Run the container

The image contains the `8ball` command-line tool and server. Supply your own models and server configuration.

- Mount a TOML configuration file as read-only, or set `EIGHTBALL_CONFIG_TOML`.
- Provide writable storage for `EIGHTBALL_DB`.
- Set `EIGHTBALL_API_KEY`.

## Helm chart

The [chart](../charts/eightball) uses the bjw-s common library and accepts values in the app-template format. The image tag defaults to the chart's appVersion, which matches the release tag.

Install the chart with the release name `eightball`. This gives the Service the address `eightball.<namespace>.svc`.

```sh
helm install eightball oci://ghcr.io/tanguille/charts/eightball --version 0.1.0
```

These options are off by default. Set an option to `true` to turn it on.

- `serviceMonitor.app.enabled` scrapes `/metrics`.
- `rawResources.prometheusRule.enabled` adds the `EightballBackendErrors` alert.
- `configMapsFromFolder.enabled` adds the dashboard as the ConfigMap `<release>-dashboard`. It uses the label `grafana_dashboard: "1"` and the data source input `DS_PROMETHEUS`.

The chart expects a ConfigMap named `<release>-config` with a `config.toml` key. It creates a 1 GiB persistent volume claim (PVC) for the database. `helm uninstall` keeps that PVC. To use an existing PVC, set `persistence.data.existingClaim`.

To pass the configuration as an environment variable, set `persistence.config.enabled: false`. In `controllers.eightball.containers.app.env`, set `EIGHTBALL_CONFIG` to `null`. Put the TOML text in `EIGHTBALL_CONFIG_TOML` in that same section.

## Release builds

The [container workflow](../.github/workflows/image.yaml) builds for `linux/amd64`, runs a smoke test, and publishes to GHCR. It uses `GITHUB_TOKEN` as its publishing token.

A push to `main` releases a new patch version if it changes code, Docker build inputs, or the chart. A push that only changes docs or this workflow leaves the release alone. The workflow moves `latest` and gives the image, chart, and Git tag the next patch version, starting with `v0.1.0` and then `v0.1.1`. GitHub generates the release notes.

The image is `ghcr.io/tanguille/8-ball:<version>`. The chart is `oci://ghcr.io/tanguille/charts/eightball` and uses the version without the `v`. Keep both packages public so that deployments and Renovate can read them without credentials.

For a minor or major release, push the tag `vX.Y.0` on the last commit of `main`. Then run `gh workflow run image.yaml --ref main`. The workflow releases the tag and uses it as the starting point for the next release. If a release fails halfway through, run the same command to finish it.

To build another branch, run `gh workflow run image.yaml --ref <branch>`. That build publishes only the `sha-` image.
