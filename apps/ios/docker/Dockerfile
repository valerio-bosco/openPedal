# Swift 6.4 + xtool, to build the iPad app from any Linux distribution
# (NixOS ships an older Swift). Used by apps/ios/scripts/xtool.sh.
FROM swift:6.4-noble

ARG XTOOL_VERSION=1.24.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git xz-utils unzip zip \
 && rm -rf /var/lib/apt/lists/*

# The AppImage is extracted, so FUSE is not needed inside the container.
RUN curl -fL "https://github.com/xtool-org/xtool/releases/download/${XTOOL_VERSION}/xtool-x86_64.AppImage" -o /tmp/xtool.AppImage \
 && chmod +x /tmp/xtool.AppImage \
 && cd /opt && /tmp/xtool.AppImage --appimage-extract >/dev/null \
 && mv squashfs-root xtool && rm /tmp/xtool.AppImage \
 && chmod -R a+rX /opt/xtool \
 && ln -s /opt/xtool/AppRun /usr/local/bin/xtool
