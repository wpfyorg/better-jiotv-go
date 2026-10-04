FROM scratch
ARG VARIANT=full
LABEL org.opencontainers.image.title="JioTV" \
      org.opencontainers.image.description="JioTV streaming and IPTV server (${VARIANT} build)" \
      org.opencontainers.image.source="https://github.com/wpfyorg/better-jiotv-go"
WORKDIR /app
ENV HOME=/app
COPY jiotv /usr/local/bin/jiotv
VOLUME ["/app/.jiotv_go"]
# 5001: plain HTTP (IPTV apps); 5443: HTTPS with a self-signed certificate stored in the data volume.
EXPOSE 5001 5443
ENTRYPOINT ["/usr/local/bin/jiotv"]
CMD ["--skip-update-check", "serve", "--host", "0.0.0.0", "--port", "5001", "--tls", "--tls-port", "5443"]
