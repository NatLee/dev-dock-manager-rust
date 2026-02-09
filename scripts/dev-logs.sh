#!/bin/bash
# Follow backend container logs (Ctrl+C to stop)
docker logs -f d-gui-manager-backend "$@"
