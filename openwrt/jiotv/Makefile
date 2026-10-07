include $(TOPDIR)/rules.mk

PKG_NAME:=jiotv
PKG_VERSION?=1.3.3
PKG_RELEASE:=1
PKGARCH:=$(ARCH_PACKAGES)

include $(INCLUDE_DIR)/package.mk

define Package/jiotv
  SECTION:=net
  CATEGORY:=Network
  TITLE:=JioTV server with web UI
  URL:=https://github.com/wpfyorg/better-jiotv-go
  CONFLICTS:=jiotv-slim
endef

define Package/jiotv/description
 JioTV streaming and IPTV server, including the embedded web UI.
endef

define Package/jiotv-slim
  SECTION:=net
  CATEGORY:=Network
  TITLE:=JioTV headless server
  URL:=https://github.com/wpfyorg/better-jiotv-go
  CONFLICTS:=jiotv
endef

define Package/jiotv-slim/description
 Headless JioTV streaming and IPTV server without the embedded web UI.
endef

define Build/Compile
endef

define Package/jiotv/conffiles
/etc/config/jiotv
endef

define Package/jiotv-slim/conffiles
/etc/config/jiotv
endef

define Package/jiotv/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/etc/init.d $(1)/etc/config $(1)/etc/uci-defaults
	$(INSTALL_BIN) $(JIOTV_FULL_BIN) $(1)/usr/bin/jiotv
	$(INSTALL_BIN) ./files/jiotv.init $(1)/etc/init.d/jiotv
	$(INSTALL_CONF) ./files/jiotv.config $(1)/etc/config/jiotv
	$(INSTALL_BIN) ./files/jiotv.defaults $(1)/etc/uci-defaults/99-jiotv
endef

define Package/jiotv-slim/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/etc/init.d $(1)/etc/config $(1)/etc/uci-defaults
	$(INSTALL_BIN) $(JIOTV_SLIM_BIN) $(1)/usr/bin/jiotv
	$(INSTALL_BIN) ./files/jiotv.init $(1)/etc/init.d/jiotv
	$(INSTALL_CONF) ./files/jiotv.config $(1)/etc/config/jiotv
	$(INSTALL_BIN) ./files/jiotv.defaults $(1)/etc/uci-defaults/99-jiotv
endef

$(eval $(call BuildPackage,jiotv))
$(eval $(call BuildPackage,jiotv-slim))
