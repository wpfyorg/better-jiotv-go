FROM scratch
ARG VARIANT=full
LABEL org.opencontainers.image.title="JioTV" \
      org.opencontainers.image.description="JioTV streaming and IPTV server (${VARIANT} build)" \
      org.opencontainers.image.source="https://github.com/wpfyorg/better-jiotv-go"
WORKDIR /app
ENV HOME=/app
COPY jiotv /usr/local/bin/jiotv
VOLUME ["/app/.jiotv_go"]
EXPOSE 5001
ENTRYPOINT ["/usr/local/bin/jiotv"]
CMD ["--skip-update-check", "serve", "--host", "0.0.0.0", "--port", "5001"]
