(() => {
  const STATE_PATTERN =
    /^ap1\.([dls])(?:\.([\dA-Fa-f]{6})\.([\dA-Fa-f]{6}))?\.[\da-f]{64,}$/u;
  const MODE_NAMES = { d: "dark", l: "light", s: "system" };
  const DARK_MODE_CLASS = "pf-v5-theme-dark";
  const DARK_TEXT = "#0F172A";
  const LIGHT_TEXT = "#FFFFFF";

  function decodeClientData(value) {
    try {
      const base64 = value.replaceAll("-", "+").replaceAll("_", "/");
      const padded = base64.padEnd(
        base64.length + ((4 - (base64.length % 4)) % 4),
        "=",
      );
      const parsed = JSON.parse(globalThis.atob(padded));
      return typeof parsed.st === "string" ? parsed.st : null;
    } catch {
      return null;
    }
  }

  function authorizationState() {
    const params = new URL(globalThis.location.href).searchParams;
    const direct = params.get("state");
    if (direct !== null) return direct;
    const clientData = params.get("client_data");
    return clientData === null ? null : decodeClientData(clientData);
  }

  function channel(value) {
    const normalized = value / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  }

  function luminance(color) {
    const [red, green, blue] = [1, 3, 5].map((start) =>
      channel(Number.parseInt(color.slice(start, start + 2), 16)),
    );
    return red * 0.2126 + green * 0.7152 + blue * 0.0722;
  }

  function readableText(background) {
    const value = luminance(background);
    const darkContrast = (value + 0.05) / 0.05;
    const lightContrast = 1.05 / (value + 0.05);
    return darkContrast >= lightContrast ? DARK_TEXT : LIGHT_TEXT;
  }

  function applyColors(root, primary, secondary) {
    root.style.setProperty("--baukit-auth-primary", primary);
    root.style.setProperty("--baukit-auth-secondary", secondary);
    root.style.setProperty("--baukit-auth-on-primary", readableText(primary));
  }

  // keycloak.v2 follows prefers-color-scheme through its own module script,
  // so an explicit mode re-applies itself whenever the class list changes.
  function pinDarkMode(root, dark) {
    const apply = () => {
      if (root.classList.contains(DARK_MODE_CLASS) !== dark) {
        root.classList.toggle(DARK_MODE_CLASS, dark);
      }
    };
    apply();
    new MutationObserver(apply).observe(root, {
      attributes: true,
      attributeFilter: ["class"],
    });
  }

  const match = authorizationState()?.match(STATE_PATTERN);
  if (!match) return;

  const root = document.documentElement;
  const [, modeCode, primary, secondary] = match;
  if (primary && secondary) {
    applyColors(
      root,
      `#${primary.toUpperCase()}`,
      `#${secondary.toUpperCase()}`,
    );
  }

  const mode = MODE_NAMES[modeCode];
  if (mode === "system") return;
  root.dataset.baukitTheme = mode;
  pinDarkMode(root, mode === "dark");
})();
