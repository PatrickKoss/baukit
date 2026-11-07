declare global {
  interface ImportMetaEnv {
    readonly VITE_API_URL?: string;
{% if context.auth_oidc %}    readonly VITE_OIDC_ISSUER?: string;
    readonly VITE_OIDC_CLIENT_ID?: string;
    readonly VITE_OIDC_AUDIENCE?: string;
    readonly VITE_OIDC_RESOURCE?: string;
    readonly VITE_OIDC_SCOPES?: string;
    readonly VITE_OIDC_OFFLINE_ACCESS?: string;
{% elif context.auth_enabled %}    readonly VITE_LOCAL_OIDC_ISSUER?: string;
{% if context.auth_clerk %}    readonly VITE_CLERK_PUBLISHABLE_KEY?: string;
{% else %}    readonly VITE_WORKOS_CLIENT_ID?: string;
{% endif %}{% endif %}  }
}

export {};
