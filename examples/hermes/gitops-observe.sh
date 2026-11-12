#!/usr/bin/env bash
# Hermes pre-check launcher: runs `8ball observe` for the gitops-investigate spec.
exec 8ball observe --spec gitops-investigate "$@"
