export const PRODUCT_NAME = '{{ context.app_name }}';
/** The realm's mobile client, `[auth] mobile_client_id` in `baukit.toml`. */
export const MOBILE_OIDC_CLIENT_ID = '{{ context.mobile_client_id }}';
/** The OIDC callback path after `<scheme>://`, `[auth] mobile_redirect_path` in `baukit.toml`. */
export const OIDC_REDIRECT_PATH = '{{ context.mobile_redirect_path }}';
