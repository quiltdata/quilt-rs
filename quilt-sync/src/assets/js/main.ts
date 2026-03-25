import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { createJSONEditor } from "vanilla-jsoneditor/standalone.js";

type Namespace = string;

type Html = string;

const DISMISS_DURATION_MS = 5 * 60 * 1000;
const STORAGE_KEY_UPDATE_DISMISSED_AT = "updateDismissedAt";

const SELECTOR_ERASE_AUTH = ".js-erase-auth";
const SELECTOR_DEBUG_DOT_QUILT = ".js-debug-dot-quilt";
const SELECTOR_DEBUG_LOGS = ".js-debug-logs";
const SELECTOR_DIRECTORY_INPUT_HINT = ".js-hint";
const SELECTOR_ENTRIES_CHECKBOX = ".js-entries-checkbox:not(:disabled)";
const SELECTOR_ENTRIES_INSTALL_SELECTED = ".js-entries-install";
const SELECTOR_ENTRIES_SELECT_ALL = ".js-entries-select-all";
const SELECTOR_LAYOUT = "#layout";
const SELECTOR_LOGIN = ".js-login";
const SELECTOR_LOGIN_OAUTH = ".js-login-oauth";
const SELECTOR_METADATA = ".js-metadata";
const SELECTOR_METADATA_INPUT = "#metadata";
const SELECTOR_NOTIFY = "#notify";
const SELECTOR_NOTIFY_SUCCESS = ".js-success";
const SELECTOR_ORIGIN_CANCEL = ".js-origin-cancel";
const SELECTOR_ORIGIN_HINT = ".js-origin-hint";
const SELECTOR_ORIGIN_INPUT = ".js-origin-input";
const SELECTOR_ORIGIN_SUBMIT = ".js-origin-submit";
const SELECTOR_UPDATE_DISMISS = ".js-update-dismiss";
const SELECTOR_UPDATE_DOWNLOAD = ".js-update-download";
const SELECTOR_UPDATE_INSTALL = ".js-update-install";
const SELECTOR_OPEN_DIRECTORY_PICKER = ".js-open-directory-picker";
const SELECTOR_OPEN_IN_DEFAULT_APPLICATION = ".js-open-in-default-application";
const SELECTOR_OPEN_IN_FILE_BROWSER = ".js-open-in-file-browser";
const SELECTOR_OPEN_IN_WEB_BROWSER = ".js-open-in-web-browser";
const SELECTOR_PACKAGE_CERTIFY_LATEST = ".js-packages-certify-latest";
const SELECTOR_PACKAGE_COMMIT = ".js-packages-commit";
const SELECTOR_OPEN_COMMIT_PAGE = ".qui-actionbar button";
const SELECTOR_PACKAGE_INSTALL = ".js-packages-install";
const SELECTOR_PACKAGE_PULL = ".js-packages-pull";
const SELECTOR_PACKAGE_PUSH = ".js-packages-push";
const SELECTOR_PACKAGE_RESET_LOCAL = ".js-packages-reset-local";
const SELECTOR_PACKAGE_UNINSTALL = ".js-packages-uninstall";
const SELECTOR_PATHS_INSTALL = ".js-paths-install";
const SELECTOR_REVEAL_IN_FILE_BROWSER = ".js-reveal-in-file-browser";
const SELECTOR_SET_ORIGIN = ".js-set-origin";
const SELECTOR_SETUP = ".js-setup";
const SELECTOR_WORKFLOW_NULL = ".js-workflow-null";
const SELECTOR_WORKFLOW_VALUE = ".js-workflow-value";
const SELECTOR_REFRESH = ".js-refresh";
const SELECTOR_COLLECT_LOGS = ".js-collect-logs";
const SELECTOR_CRASH_REPORT = ".js-crash-report";
const SELECTOR_DIAGNOSTIC_LOGS = ".js-diagnostic-logs";
const SELECTOR_OPEN_HOME_DIR = ".js-open-home-dir";
const SELECTOR_OPEN_DATA_DIR = ".js-open-data-dir";

const I18N = {
  collectingLogs: "Collecting\u2026",
  logsCollected: "Logs collected:",
  revealFile: "Reveal",
} as const;

type SELECTOR_FORM = "#form";

type SELECTOR_DIRECTORY_INPUT = "#input";

type Selector =
  | SELECTOR_DIRECTORY_INPUT
  | SELECTOR_FORM
  | typeof SELECTOR_ERASE_AUTH
  | typeof SELECTOR_DEBUG_DOT_QUILT
  | typeof SELECTOR_DEBUG_LOGS
  | typeof SELECTOR_DIRECTORY_INPUT_HINT
  | typeof SELECTOR_ENTRIES_CHECKBOX
  | typeof SELECTOR_ENTRIES_INSTALL_SELECTED
  | typeof SELECTOR_ENTRIES_SELECT_ALL
  | typeof SELECTOR_LAYOUT
  | typeof SELECTOR_LOGIN
  | typeof SELECTOR_LOGIN_OAUTH
  | typeof SELECTOR_METADATA
  | typeof SELECTOR_METADATA_INPUT
  | typeof SELECTOR_NOTIFY
  | typeof SELECTOR_NOTIFY_SUCCESS
  | typeof SELECTOR_OPEN_COMMIT_PAGE
  | typeof SELECTOR_OPEN_DIRECTORY_PICKER
  | typeof SELECTOR_OPEN_IN_DEFAULT_APPLICATION
  | typeof SELECTOR_OPEN_IN_FILE_BROWSER
  | typeof SELECTOR_OPEN_IN_WEB_BROWSER
  | typeof SELECTOR_ORIGIN_CANCEL
  | typeof SELECTOR_ORIGIN_HINT
  | typeof SELECTOR_ORIGIN_INPUT
  | typeof SELECTOR_ORIGIN_SUBMIT
  | typeof SELECTOR_PACKAGE_CERTIFY_LATEST
  | typeof SELECTOR_PACKAGE_COMMIT
  | typeof SELECTOR_PACKAGE_INSTALL
  | typeof SELECTOR_PACKAGE_PULL
  | typeof SELECTOR_PACKAGE_PUSH
  | typeof SELECTOR_PACKAGE_RESET_LOCAL
  | typeof SELECTOR_PACKAGE_UNINSTALL
  | typeof SELECTOR_PATHS_INSTALL
  | typeof SELECTOR_REFRESH
  | typeof SELECTOR_REVEAL_IN_FILE_BROWSER
  | typeof SELECTOR_SET_ORIGIN
  | typeof SELECTOR_SETUP
  | typeof SELECTOR_UPDATE_DISMISS
  | typeof SELECTOR_UPDATE_DOWNLOAD
  | typeof SELECTOR_UPDATE_INSTALL
  | typeof SELECTOR_WORKFLOW_NULL
  | typeof SELECTOR_WORKFLOW_VALUE
  | typeof SELECTOR_COLLECT_LOGS
  | typeof SELECTOR_CRASH_REPORT
  | typeof SELECTOR_DIAGNOSTIC_LOGS
  | typeof SELECTOR_OPEN_HOME_DIR
  | typeof SELECTOR_OPEN_DATA_DIR;

const CMD_ERASE_AUTH = "erase_auth";
const CMD_DEBUG_DOT_QUILT = "debug_dot_quilt";
const CMD_DEBUG_LOGS = "debug_logs";
const CMD_LOGIN = "login";
const CMD_LOGIN_OAUTH = "login_oauth";
const CMD_OPEN_DIRECTORY_PICKER = "open_directory_picker";
const CMD_OPEN_IN_DEFAULT_APPLICATION = "open_in_default_application";
const CMD_OPEN_IN_FILE_BROWSER = "open_in_file_browser";
const CMD_OPEN_IN_WEB_BROWSER = "open_in_web_browser";
const CMD_PACKAGE_CERTIFY_LATEST = "certify_latest";
const CMD_PACKAGE_COMMIT = "package_commit";
const CMD_PACKAGE_INSTALL = "package_install";
const CMD_PACKAGE_PULL = "package_pull";
const CMD_PACKAGE_PUSH = "package_push";
const CMD_PACKAGE_RESET_LOCAL = "reset_local";
const CMD_PACKAGE_UNINSTALL = "package_uninstall";
const CMD_REVEAL_IN_FILE_BROWSER = "reveal_in_file_browser";
const CMD_SET_ORIGIN = "set_origin";
const CMD_SETUP = "setup";
const CMD_COLLECT_LOGS = "collect_diagnostic_logs";
const CMD_CRASH_REPORT = "send_crash_report";
const CMD_OPEN_HOME_DIR = "open_home_dir";
const CMD_OPEN_DATA_DIR = "open_data_dir";

type Command =
  | typeof CMD_ERASE_AUTH
  | typeof CMD_DEBUG_DOT_QUILT
  | typeof CMD_DEBUG_LOGS
  | typeof CMD_LOGIN
  | typeof CMD_LOGIN_OAUTH
  | typeof CMD_OPEN_DIRECTORY_PICKER
  | typeof CMD_OPEN_IN_DEFAULT_APPLICATION
  | typeof CMD_OPEN_IN_FILE_BROWSER
  | typeof CMD_OPEN_IN_WEB_BROWSER
  | typeof CMD_PACKAGE_CERTIFY_LATEST
  | typeof CMD_PACKAGE_COMMIT
  | typeof CMD_PACKAGE_INSTALL
  | typeof CMD_PACKAGE_PULL
  | typeof CMD_PACKAGE_PUSH
  | typeof CMD_PACKAGE_RESET_LOCAL
  | typeof CMD_PACKAGE_UNINSTALL
  | typeof CMD_REVEAL_IN_FILE_BROWSER
  | typeof CMD_SET_ORIGIN
  | typeof CMD_SETUP
  | typeof CMD_COLLECT_LOGS
  | typeof CMD_CRASH_REPORT
  | typeof CMD_OPEN_HOME_DIR
  | typeof CMD_OPEN_DATA_DIR;

function handleError(e: Error | unknown) {
  if (e instanceof Error) {
    notify(e.message);
  } else if (typeof e === "string") {
    notify(e);
  } else {
    notify(`${e}`);
  }

  unlockUI();
}

const ROUTE_INSTALLED_PACKAGES_LIST = "installed-packages-list.html";
const ROUTE_INSTALLED_PACKAGE = (namespace: Namespace) =>
  `installed-package.html#namespace=${namespace}`;
const ROUTE_LOGIN = (host: string, location: string) =>
  `login.html#host=${host}&location=${encodeURIComponent(location)}`;
const ROUTE_REMOTE_PACKAGE = (uri: string) =>
  `remote-package.html?uri=${encodeURIComponent(uri)}`;

type Route =
  | typeof ROUTE_INSTALLED_PACKAGES_LIST
  | ReturnType<typeof ROUTE_INSTALLED_PACKAGE>
  | ReturnType<typeof ROUTE_LOGIN>
  | ReturnType<typeof ROUTE_REMOTE_PACKAGE>;

const EVENT_PAGE_READY = "page-is-ready";

function findElement(selector: Selector, optParent?: Element) {
  return (optParent ?? document).querySelector(selector);
}

function findElementsList(selector: Selector, optParent?: Element) {
  return (optParent ?? document).querySelectorAll(selector);
}

function assertElementHasDataAttributes(element: Element, attrs: string[]) {
  for (const attr of attrs) {
    if (!element.hasAttribute(`data-${attr}`)) {
      throw new Error(`Element is missing data attribute: ${attr}`);
    }
  }
}

function getCommandDataFromDataAttributes<T extends string>(
  element: EventTarget | null,
  attrs: T[],
) {
  if (!element) {
    throw new Error("Element is missing");
  }
  return attrs.reduce(
    (memo, attr) => {
      const value = (element as Element).getAttribute(`data-${attr}`);
      if (!value) {
        throw new Error("Attribute value is missing");
      }
      memo[attr] = value;
      return memo;
    },
    {} as Record<T, string>,
  );
}

function lockUI() {
  const layout = findElement(SELECTOR_LAYOUT);
  layout?.setAttribute("disabled", "disabled");
}

function unlockUI() {
  const layout = findElement(SELECTOR_LAYOUT);
  layout?.removeAttribute("disabled");
}

function notify(html: Html) {
  const outputElement = findElement(SELECTOR_NOTIFY);
  if (!outputElement) return;
  outputElement.innerHTML = html;

  if (findElement(SELECTOR_NOTIFY_SUCCESS, outputElement)) {
    setTimeout(() => {
      outputElement.innerHTML = "";
    }, 3000);
  }

  if (outputElement.querySelector(SELECTOR_NOTIFY_SUCCESS)) {
    return 1;
  }
  return 0;
}

function onCheckbox() {
  const submitInstall = findElement(SELECTOR_ENTRIES_INSTALL_SELECTED);
  const checkboxes: NodeListOf<HTMLInputElement> = document.querySelectorAll(
    SELECTOR_ENTRIES_CHECKBOX,
  );
  if (!submitInstall || !checkboxes.length) {
    return;
  }
  const checkedCheckboxes = Array.from(checkboxes)
    .map((el) => el.checked && !el.disabled)
    .filter(Boolean);

  const commitButton = findElement(SELECTOR_OPEN_COMMIT_PAGE);
  commitButton?.classList.toggle("primary", !checkedCheckboxes.length);
  submitInstall?.classList.toggle("primary", !!checkedCheckboxes.length);

  if (checkedCheckboxes.length) {
    submitInstall.removeAttribute("disabled");
  } else {
    submitInstall.setAttribute("disabled", "disabled");
  }
  const selectAllEl = findElement(SELECTOR_ENTRIES_SELECT_ALL);
  if (selectAllEl) {
    (selectAllEl as HTMLInputElement).removeAttribute("disabled");
    (selectAllEl as HTMLInputElement).checked =
      checkedCheckboxes.length === checkboxes.length;
  }
}

function isNodeList(el: Element | NodeList): el is NodeList {
  return !!(el as NodeList).length;
}

async function installPaths(event: SubmitEvent) {
  event.preventDefault();
  const pathsEls = (event.currentTarget as HTMLFormElement).elements.namedItem(
    "path",
  );
  if (!pathsEls) {
    throw new Error("Element not found");
  }
  const pathsElsList = isNodeList(pathsEls)
    ? Array.from(pathsEls)
    : [pathsEls as HTMLInputElement];
  const uriEl = (event.currentTarget as HTMLFormElement).elements.namedItem(
    "uri",
  );
  if (!uriEl) {
    throw new Error("Element not found");
  }
  const formData = {
    paths: pathsElsList
      .map((element) => {
        const el = element as HTMLInputElement;
        return !el.disabled && el.checked && el.value;
      })
      .filter(Boolean),
    uri: (uriEl as HTMLInputElement).value,
  };
  lockUI();
  const notifications: Html = await invoke("package_install_paths", formData);
  const namespace = formData.uri.match(/#package=(.*)@/)?.[1];
  if (namespace) {
    navigate(ROUTE_INSTALLED_PACKAGE(namespace));
  }
  unlockUI();
  notify(notifications);
}

export function selectAllPaths(targetElement: HTMLInputElement) {
  const form = targetElement.closest("form");
  if (!form) return;
  const checked = targetElement.checked;
  const checkboxes = findElementsList(SELECTOR_ENTRIES_CHECKBOX, form);
  for (const checkbox of checkboxes as NodeListOf<HTMLInputElement>) {
    if (!checkbox.disabled) {
      (checkbox as HTMLInputElement).checked = checked;
    }
  }
  onCheckbox();
}

async function loadCurrentPage() {
  const page: Html = await invoke("load_page", {
    location: window.location.href,
  });
  document.body.innerHTML = page;

  window.dispatchEvent(new Event(EVENT_PAGE_READY));

  return Promise.resolve(null);
}

async function navigate(url: Route) {
  if (window.location.href.endsWith(url)) {
    window.location.reload();
  } else {
    window.location.assign(url);
  }
  // `loadCurrentPage` is triggered on `DOMContentLoaded` on Linux
  // But it should not!
  // So, let just reload the page and be sure the behaviour is consistent and foolproof
  // window.history.pushState({}, "", url);
  // return loadCurrentPage();
  return Promise.resolve(null);
}

async function execPageCommand<T extends string>(
  command: Command,
  data: Record<T, string>,
  optRedirect?: Route,
) {
  const layout = findElement("#layout");
  layout?.setAttribute("disabled", "disabled");
  const notification: Html = await invoke(command, data);
  layout?.removeAttribute("disabled");

  if (!notify(notification)) {
    return;
  }

  if (optRedirect) {
    await navigate(optRedirect);
  } else {
    await loadCurrentPage();
  }
}

function collectFormData(formSelector: SELECTOR_FORM) {
  const formElement = findElement(formSelector);
  if (!formElement) {
    throw new Error("Form not found");
  }
  const form = new FormData(formElement as HTMLFormElement);
  const formData: Record<string, string> = {};
  for (const [key, value] of form.entries()) {
    formData[key] = value as string;
  }
  return formData;
}

async function execFormCommand(
  command: Command,
  formData: Record<string, string>,
) {
  const layout = findElement("#layout");
  layout?.setAttribute("disabled", "disabled");

  const notification: Html = await invoke(command, formData);
  layout?.removeAttribute("disabled");

  if (!notify(notification)) {
    return;
  }

  if (formData.namespace) {
    await navigate(ROUTE_INSTALLED_PACKAGE(formData.namespace));
  } else {
    await navigate(ROUTE_INSTALLED_PACKAGES_LIST);
  }
}

async function pickupDirectory(inputSelector: SELECTOR_DIRECTORY_INPUT) {
  const inputEl = findElement(inputSelector);
  if (!inputEl) {
    notify("Input element not found");
    return;
  }

  lockUI();

  const hint =
    inputEl.parentElement &&
    findElement(SELECTOR_DIRECTORY_INPUT_HINT, inputEl.parentElement);
  try {
    const directory: string = await invoke(CMD_OPEN_DIRECTORY_PICKER);
    (inputEl as HTMLInputElement).value = directory || "";
    hint?.firstChild?.remove();
  } catch (error) {
    hint?.appendChild(new Text(error as string));
  }

  unlockUI();
}

async function execInlineCommand(
  command: Command,
  data: Record<string, string>,
  button: HTMLButtonElement,
) {
  button.setAttribute("disabled", "disabled");
  const notification: Html = await invoke(command, data);
  button.removeAttribute("disabled");
  notify(notification);
}

function listen<T extends string>(
  selector: Selector,
  attrs: T[],
  callback: (
    data: Record<T, string>,
    button: HTMLButtonElement,
  ) => Promise<void>,
) {
  for (const element of findElementsList(selector)) {
    assertElementHasDataAttributes(element, attrs);
    element.addEventListener("click", (event) => {
      const button = event.currentTarget as HTMLButtonElement;
      try {
        const command = getCommandDataFromDataAttributes(element, attrs);
        callback(command, button).catch(handleError);
      } catch (error) {
        handleError(error);
      }
    });
  }
}

/** Stored zip path after "Collect Logs" succeeds. */
let collectedZipPath: string | null = null;

function showCollectLogsResult(zipPath: string) {
  const resultDiv = document.getElementById("collect-logs-result");
  if (!resultDiv) return;

  resultDiv.hidden = false;
  resultDiv.innerHTML = `<span class="zip-path-label">${I18N.logsCollected}</span>
    <code></code>
    <button class="qui-button link js-file-reveal small" type="button"><img class="qui-icon" src="/assets/img/icons/folder_open.svg" /><span>${I18N.revealFile}</span></button>`;
  resultDiv.querySelector("code")!.textContent = zipPath;

  resultDiv.querySelector(".js-file-reveal")?.addEventListener("click", async () => {
    const sep = Math.max(zipPath.lastIndexOf("/"), zipPath.lastIndexOf("\\"));
    const dir = sep > 0 ? zipPath.substring(0, sep) : zipPath;
    await invoke(CMD_OPEN_IN_WEB_BROWSER, { url: dir });
  });
}

function enableDeliveryButtons() {
  const container = document.getElementById("diagnostic-actions");
  if (!container) return;
  for (const selector of [SELECTOR_CRASH_REPORT, SELECTOR_DIAGNOSTIC_LOGS] as Selector[]) {
    const btn = container.querySelector(selector) as HTMLButtonElement | null;
    btn?.removeAttribute("disabled");
  }
}

window.addEventListener(EVENT_PAGE_READY, () => {
  listen(SELECTOR_ERASE_AUTH, ["host"], (data, button) =>
    execInlineCommand(CMD_ERASE_AUTH, data, button).then(() => {
      navigate(ROUTE_LOGIN(data.host, window.location.href));
    }),
  );
  listen(SELECTOR_DEBUG_DOT_QUILT, [], (data, button) =>
    execInlineCommand(CMD_DEBUG_DOT_QUILT, data, button),
  );

  listen(SELECTOR_DEBUG_LOGS, [], (data, button) =>
    execInlineCommand(CMD_DEBUG_LOGS, data, button),
  );

  listen(SELECTOR_COLLECT_LOGS, [], async (_data, button) => {
    const originalLabel = button.querySelector("span")?.textContent ?? "";
    button.setAttribute("disabled", "disabled");
    const span = button.querySelector("span");
    if (!span) {
      handleError("Missing text element inside collect logs button");
      return;
    }
    span.textContent = I18N.collectingLogs;
    try {
      const zipPath: string = await invoke(CMD_COLLECT_LOGS);
      collectedZipPath = zipPath;
      showCollectLogsResult(zipPath);
      enableDeliveryButtons();
    } catch (error) {
      handleError(error);
    } finally {
      button.removeAttribute("disabled");
      span.textContent = originalLabel;
    }
  });

  listen(SELECTOR_CRASH_REPORT, [], async (_data, button) => {
    if (!collectedZipPath) {
      console.warn("Crash report requested but no diagnostic zip collected yet");
      return;
    }
    button.setAttribute("disabled", "disabled");
    try {
      const notification: Html = await invoke(CMD_CRASH_REPORT, { zipPath: collectedZipPath });
      notify(notification);
    } catch (error) {
      handleError(error);
    } finally {
      button.removeAttribute("disabled");
    }
  });

  listen(SELECTOR_DIAGNOSTIC_LOGS, ["version", "os"], async (data) => {
    if (!collectedZipPath) {
      console.warn("Email support requested but no diagnostic zip collected yet");
      return;
    }
    const subject = encodeURIComponent(
      `Quilt issue report (v${data["version"]}, ${data["os"]})`,
    );
    const body = encodeURIComponent(
      `Please describe the issue:\n...\n\nDiagnostic logs saved to:\n${collectedZipPath}\nPlease attach this file to this email.`,
    );
    const mailtoUrl = `mailto:support@quilt.bio?subject=${subject}&body=${body}`;
    await invoke(CMD_OPEN_IN_WEB_BROWSER, { url: mailtoUrl });
  });

  listen(SELECTOR_OPEN_HOME_DIR, [], (data, button) =>
    execInlineCommand(CMD_OPEN_HOME_DIR, data, button),
  );

  listen(SELECTOR_OPEN_DATA_DIR, [], (data, button) =>
    execInlineCommand(CMD_OPEN_DATA_DIR, data, button),
  );

  listen(SELECTOR_PACKAGE_INSTALL, ["uri"], (data) =>
    execPageCommand(CMD_PACKAGE_INSTALL, data, ROUTE_INSTALLED_PACKAGES_LIST),
  );

  listen(SELECTOR_PACKAGE_UNINSTALL, ["namespace"], (data) =>
    execPageCommand(CMD_PACKAGE_UNINSTALL, data, ROUTE_INSTALLED_PACKAGES_LIST),
  );

  listen(SELECTOR_SET_ORIGIN, ["namespace"], async (data, button) => {
    const currentOrigin = button.getAttribute("data-origin") ?? "";
    showSetOriginForm(data.namespace, currentOrigin);
  });

  listen(SELECTOR_PACKAGE_CERTIFY_LATEST, ["namespace"], (data) =>
    execPageCommand(
      CMD_PACKAGE_CERTIFY_LATEST,
      data,
      ROUTE_INSTALLED_PACKAGE(data.namespace),
    ),
  );

  listen(SELECTOR_PACKAGE_RESET_LOCAL, ["namespace"], (data) =>
    execPageCommand(
      CMD_PACKAGE_RESET_LOCAL,
      data,
      ROUTE_INSTALLED_PACKAGE(data.namespace),
    ),
  );

  listen(SELECTOR_PACKAGE_PULL, ["namespace"], (data) =>
    execPageCommand(
      CMD_PACKAGE_PULL,
      data,
      ROUTE_INSTALLED_PACKAGE(data.namespace),
    ),
  );

  listen(SELECTOR_PACKAGE_PUSH, ["namespace"], (data) =>
    execPageCommand(CMD_PACKAGE_PUSH, data),
  );

  listen(SELECTOR_OPEN_IN_FILE_BROWSER, ["namespace"], (data, button) =>
    execInlineCommand(CMD_OPEN_IN_FILE_BROWSER, data, button),
  );

  listen(SELECTOR_OPEN_IN_WEB_BROWSER, ["url"], (data, button) =>
    execInlineCommand(CMD_OPEN_IN_WEB_BROWSER, data, button),
  );

  listen(
    SELECTOR_REVEAL_IN_FILE_BROWSER,
    ["namespace", "path"],
    (data, button) =>
      execInlineCommand(CMD_REVEAL_IN_FILE_BROWSER, data, button),
  );

  listen(SELECTOR_OPEN_DIRECTORY_PICKER, ["target"], (data) =>
    pickupDirectory(data.target as SELECTOR_DIRECTORY_INPUT),
  );

  listen(
    SELECTOR_OPEN_IN_DEFAULT_APPLICATION,
    ["namespace", "path"],
    (data, button) =>
      execInlineCommand(CMD_OPEN_IN_DEFAULT_APPLICATION, data, button),
  );

  listen(SELECTOR_PACKAGE_COMMIT, ["form"], (data) =>
    execFormCommand(
      CMD_PACKAGE_COMMIT,
      collectFormData(data.form as SELECTOR_FORM),
    ),
  );

  // TODO: SELECTOR_LOGIN uses inline invoke + manual layout/navigation handling
  // while SELECTOR_LOGIN_OAUTH uses execInlineCommand. Align both handlers to
  // the same pattern once the behavioral differences are understood.
  listen(SELECTOR_LOGIN, ["form"], async (data) => {
    const formData = collectFormData(data.form as SELECTOR_FORM);
    const layout = findElement("#layout");
    layout?.setAttribute("disabled", "disabled");
    const notification: Html = await invoke(CMD_LOGIN, formData);
    layout?.removeAttribute("disabled");
    if (!notify(notification)) return;
    // Rust's login_command calls navigate_after_login when location is present;
    // only navigate from JS when there is no location to avoid a double navigation.
    if (!formData.location) {
      await navigate(ROUTE_INSTALLED_PACKAGES_LIST);
    }
  });

  listen(SELECTOR_LOGIN_OAUTH, ["host"], (data, button) => {
    const location = button.getAttribute("data-location");
    const payload = location ? { ...data, location } : data;
    return execInlineCommand(CMD_LOGIN_OAUTH, payload, button);
  });

  listen(SELECTOR_SETUP, ["form"], (data) =>
    execFormCommand(CMD_SETUP, collectFormData(data.form as SELECTOR_FORM)),
  );

  findElement(SELECTOR_WORKFLOW_NULL)?.addEventListener("change", (event) => {
    if ((event.currentTarget as HTMLInputElement).checked) {
      findElement(SELECTOR_WORKFLOW_VALUE)?.setAttribute(
        "disabled",
        "disabled",
      );
    } else {
      findElement(SELECTOR_WORKFLOW_VALUE)?.removeAttribute("disabled");
    }
  });

  findElement(SELECTOR_PATHS_INSTALL)?.addEventListener("submit", (event) =>
    installPaths(event as SubmitEvent),
  );

  const selectAllElement = findElement(
    SELECTOR_ENTRIES_SELECT_ALL,
  ) as HTMLInputElement;
  if (selectAllElement) {
    selectAllElement.addEventListener("change", (event) =>
      selectAllPaths(event.currentTarget as HTMLInputElement),
    );

    // Auto-select all checkboxes on page load
    selectAllElement.checked = true;
    selectAllPaths(selectAllElement);
  }

  for (const button of findElementsList(SELECTOR_REFRESH)) {
    button.addEventListener("click", () => {
      window.location.reload();
    });
  }

  for (const checkbox of findElementsList(SELECTOR_ENTRIES_CHECKBOX)) {
    checkbox.addEventListener("change", onCheckbox);
  }
  onCheckbox();

  const textarea = findElement(SELECTOR_METADATA_INPUT);
  if (textarea) {
    const textareaField = textarea.parentElement;
    if (textareaField) {
      textareaField.style.display = "none";
    }

    const target = findElement(SELECTOR_METADATA);
    if (target) {
      createJSONEditor({
        target,
        props: {
          content: {
            text: (textarea as HTMLTextAreaElement).value || "",
          },
          onChange: (updatedContent: { json: object; text: string }) => {
            const textarea = findElement(
              SELECTOR_METADATA_INPUT,
            ) as HTMLTextAreaElement;
            if (textarea) {
              if (updatedContent.json) {
                textarea.value = JSON.stringify(updatedContent.json);
              } else {
                textarea.value = updatedContent.text;
              }
            }
          },
          navigationBar: false,
        },
      });
    }
  }
});

function isUpdateDismissed() {
  const dismissedAt = localStorage.getItem(STORAGE_KEY_UPDATE_DISMISSED_AT);
  if (!dismissedAt) return false;
  return Date.now() - Number(dismissedAt) < DISMISS_DURATION_MS;
}

function dismissUpdate() {
  localStorage.setItem(STORAGE_KEY_UPDATE_DISMISSED_AT, String(Date.now()));
  notify("");
}

async function downloadUpdate(update: Awaited<ReturnType<typeof check>>) {
  notify('<div class="update-bar"><span>Downloading update…</span></div>');
  try {
    await update?.download();
    showInstallNotification(update);
  } catch (error) {
    notify(`<div class="error">Failed to download update: ${error}</div>`);
  }
}

async function installUpdate(update: Awaited<ReturnType<typeof check>>) {
  notify('<div class="update-bar"><span>Installing update…</span></div>');
  try {
    await update?.install();
    await relaunch();
  } catch (error) {
    notify(`<div class="error">Failed to install update: ${error}</div>`);
  }
}

const HOSTNAME_PATTERN =
  /^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?)+$/;

function isValidHostname(value: string) {
  return HOSTNAME_PATTERN.test(value);
}

function showSetOriginForm(namespace: Namespace, currentOrigin: string = "") {
  notify(`<div class="origin-form">
    <label>Catalog origin</label>
    <div class="origin-input-group">
      <input class="origin-input js-origin-input" type="text" placeholder="open.quilt.bio" />
      <span class="origin-hint js-origin-hint">Enter a valid hostname, e.g. open.quilt.bio</span>
    </div>
    <div class="origin-form-actions">
      <button class="qui-button primary js-origin-submit"><span>Submit</span></button>
      <button class="qui-button js-origin-cancel"><span>Cancel</span></button>
    </div>
  </div>`);

  const outputElement = findElement(SELECTOR_NOTIFY);
  if (!outputElement) return;

  const form = outputElement.firstElementChild;
  form?.addEventListener("click", (event) => {
    event.stopPropagation();
  });

  const input = findElement(
    SELECTOR_ORIGIN_INPUT,
    outputElement,
  ) as HTMLInputElement | null;
  if (!input) return;

  if (currentOrigin) {
    input.value = currentOrigin;
  }
  input.focus();

  const hint = findElement(SELECTOR_ORIGIN_HINT, outputElement);

  const showError = () => {
    input.classList.add("error");
    hint?.classList.add("visible");
  };

  const hideError = () => {
    input.classList.remove("error");
    hint?.classList.remove("visible");
  };

  const submit = () => {
    const origin = input.value.trim();
    if (!origin) return;
    if (!isValidHostname(origin)) {
      showError();
      return;
    }
    execPageCommand(CMD_SET_ORIGIN, { namespace, origin }).catch(handleError);
  };

  const cancel = () => {
    notify("");
  };

  const submitButton = findElement(SELECTOR_ORIGIN_SUBMIT, outputElement);
  submitButton?.addEventListener("click", submit);

  const cancelButton = findElement(SELECTOR_ORIGIN_CANCEL, outputElement);
  cancelButton?.addEventListener("click", cancel);

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      submit();
    } else if (event.key === "Escape") {
      cancel();
    }
  });

  input.addEventListener("input", hideError);
}

function showInstallNotification(update: Awaited<ReturnType<typeof check>>) {
  notify(`<div class="update-bar">
    <span>Update downloaded</span>
    <div class="update-bar-actions">
      <button class="qui-button primary js-update-install"><span>Install & Restart</span></button>
      <button class="qui-button js-update-dismiss"><span>Dismiss</span></button>
    </div>
  </div>`);

  const outputElement = findElement(SELECTOR_NOTIFY);
  if (!outputElement) return;

  const installButton = findElement(SELECTOR_UPDATE_INSTALL, outputElement);
  installButton?.addEventListener("click", () => installUpdate(update));

  const dismissButton = findElement(SELECTOR_UPDATE_DISMISS, outputElement);
  dismissButton?.addEventListener("click", dismissUpdate);
}

function showUpdateNotification(
  version: string,
  update: Awaited<ReturnType<typeof check>>,
) {
  notify(`<div class="update-bar">
    <span>Update available: ${version}</span>
    <div class="update-bar-actions">
      <button class="qui-button primary js-update-download"><span>Download</span></button>
      <button class="qui-button js-update-dismiss"><span>Dismiss</span></button>
    </div>
  </div>`);

  const outputElement = findElement(SELECTOR_NOTIFY);
  if (!outputElement) return;

  const downloadButton = findElement(SELECTOR_UPDATE_DOWNLOAD, outputElement);
  downloadButton?.addEventListener("click", () => downloadUpdate(update));

  const dismissButton = findElement(SELECTOR_UPDATE_DISMISS, outputElement);
  dismissButton?.addEventListener("click", dismissUpdate);
}

async function checkForUpdates() {
  if (isUpdateDismissed()) return;

  try {
    const update = await check();
    if (update) {
      showUpdateNotification(update.version, update);
    }
  } catch (error) {
    notify(
      `<div class="error">Failed to check for QuiltSync updates: ${error}</div>`,
    );
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  await loadCurrentPage();
  await checkForUpdates();
});
