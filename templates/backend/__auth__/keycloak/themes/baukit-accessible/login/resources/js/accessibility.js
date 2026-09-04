(() => {
  const CLIENT_ERROR_PREFIX = "baukit-client-error-";
  const INITIALIZED_ATTRIBUTE = "data-baukit-accessibility-initialized";
  const CONTROL_SELECTOR = "input, select, textarea";
  const REQUIRED_MARKER_SELECTOR =
    ".pf-v5-c-form__label-required, .pf-v6-c-form__label-required";
  const FORM_GROUP_SELECTOR = ".pf-v5-c-form__group, .pf-v6-c-form__group";

  function attributeTokens(element, name) {
    return (element.getAttribute(name) ?? "").split(/\s+/u).filter(Boolean);
  }

  function addAttributeToken(element, name, token) {
    const tokens = attributeTokens(element, name);
    if (!tokens.includes(token)) tokens.push(token);
    element.setAttribute(name, tokens.join(" "));
  }

  function removeAttributeToken(element, name, token) {
    const tokens = attributeTokens(element, name).filter(
      (value) => value !== token,
    );
    if (tokens.length === 0) element.removeAttribute(name);
    else element.setAttribute(name, tokens.join(" "));
  }

  function isVisibleControl(control) {
    return (
      !control.disabled &&
      !control.hidden &&
      control.type !== "hidden" &&
      control.getAttribute("aria-hidden") !== "true"
    );
  }

  function controlKey(control) {
    return control.id || control.name;
  }

  function isEmpty(control, form) {
    if (control.type === "checkbox") return !control.checked;
    if (control.type === "radio") {
      return !Array.from(form.querySelectorAll(CONTROL_SELECTOR)).some(
        (candidate) =>
          candidate.type === "radio" &&
          candidate.name === control.name &&
          candidate.checked,
      );
    }
    return control.value.length === 0;
  }

  function loginControls(form) {
    return ["username", "password"]
      .map((id) => document.getElementById(id))
      .filter(
        (control) =>
          control && form.contains(control) && isVisibleControl(control),
      );
  }

  function registrationControls(form) {
    return Array.from(form.querySelectorAll(CONTROL_SELECTOR)).filter(
      (control) => {
        if (!isVisibleControl(control) || control.type === "submit")
          return false;
        const group = control.closest(FORM_GROUP_SELECTOR);
        return Boolean(group?.querySelector(REQUIRED_MARKER_SELECTOR));
      },
    );
  }

  function uniqueControls(controls) {
    const keys = new Set();
    return controls.filter((control) => {
      const key = controlKey(control);
      if (!key || keys.has(key)) return false;
      keys.add(key);
      return true;
    });
  }

  function serverError(control) {
    const key = controlKey(control);
    if (!key) return null;
    const error = document.getElementById(`input-error-${key}`);
    return error?.textContent.trim() ? error : null;
  }

  function makeServerErrorLive(control) {
    const error = serverError(control);
    if (!error) return false;
    error.setAttribute("role", "alert");
    error.setAttribute("aria-live", "assertive");
    control.setAttribute("aria-invalid", "true");
    addAttributeToken(control, "aria-describedby", error.id);
    return true;
  }

  function clientErrorId(control) {
    return `${CLIENT_ERROR_PREFIX}${controlKey(control)}`;
  }

  function createClientError(control) {
    const error = document.createElement("div");
    error.id = clientErrorId(control);
    error.setAttribute("data-baukit-client-error", "true");
    error.setAttribute("role", "alert");
    error.setAttribute("aria-live", "assertive");
    error.textContent = control.validationMessage;

    const group = control.closest(FORM_GROUP_SELECTOR);
    if (group) group.append(error);
    else control.insertAdjacentElement("afterend", error);
    return error;
  }

  function initializeControl(control, form) {
    const initialInvalid = control.getAttribute("aria-invalid");
    control.required = true;
    control.setAttribute("aria-required", "true");

    function showClientError() {
      const error =
        document.getElementById(clientErrorId(control)) ??
        createClientError(control);
      error.textContent = control.validationMessage;
      control.setAttribute("aria-invalid", "true");
      addAttributeToken(control, "aria-describedby", error.id);
    }

    function clearClientError() {
      const error = document.getElementById(clientErrorId(control));
      if (!error) return;
      error.remove();
      removeAttributeToken(control, "aria-describedby", error.id);
      if (serverError(control)) control.setAttribute("aria-invalid", "true");
      else if (initialInvalid === null) control.removeAttribute("aria-invalid");
      else control.setAttribute("aria-invalid", initialInvalid);
    }

    control.addEventListener("input", () => {
      if (!isEmpty(control, form)) clearClientError();
    });

    return { control, clearClientError, showClientError };
  }

  function focusServerError(form) {
    const invalidControl = Array.from(
      form.querySelectorAll(CONTROL_SELECTOR),
    ).find(
      (control) => isVisibleControl(control) && makeServerErrorLive(control),
    );
    if (invalidControl) {
      invalidControl.focus();
      return;
    }

    const alert = document.querySelector(
      '.pf-v5-c-alert.pf-m-danger, .pf-v6-c-alert.pf-m-danger, [data-alert="danger"]',
    );
    if (!alert) return;
    alert.setAttribute("role", "alert");
    alert.setAttribute("aria-live", "assertive");
    alert.setAttribute("tabindex", "-1");
    alert.focus();
  }

  function initializeForm(form, registration) {
    if (form.hasAttribute(INITIALIZED_ATTRIBUTE)) return;
    form.setAttribute(INITIALIZED_ATTRIBUTE, "true");

    const controls = uniqueControls(
      registration ? registrationControls(form) : loginControls(form),
    ).map((control) => initializeControl(control, form));
    let firstNativeInvalid = null;
    let nativeFocusScheduled = false;

    form.addEventListener(
      "invalid",
      (event) => {
        const field = controls.find(
          (candidate) => candidate.control === event.target,
        );
        if (!field) return;
        event.preventDefault();
        field.showClientError();
        firstNativeInvalid ??= field.control;
        if (nativeFocusScheduled) return;
        nativeFocusScheduled = true;
        Promise.resolve().then(() => {
          firstNativeInvalid?.focus();
          firstNativeInvalid = null;
          nativeFocusScheduled = false;
        });
      },
      true,
    );

    form.addEventListener(
      "submit",
      (event) => {
        let firstInvalid = null;
        for (const field of controls) {
          if (isEmpty(field.control, form)) {
            field.showClientError();
            firstInvalid ??= field.control;
          } else {
            field.clearClientError();
          }
        }
        if (!firstInvalid) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        firstInvalid.focus();
      },
      true,
    );

    focusServerError(form);
  }

  function initialize() {
    const login = document.getElementById("kc-form-login");
    if (login) initializeForm(login, false);
    const registration = document.getElementById("kc-register-form");
    if (registration) initializeForm(registration, true);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initialize, { once: true });
  } else {
    initialize();
  }
})();
