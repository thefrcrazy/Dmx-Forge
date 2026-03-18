(function () {
  const root = document.documentElement;
  const body = document.body;
  const sidebar = document.querySelector("[data-sidebar]");
  const sidebarToggle = document.querySelector("[data-sidebar-toggle]");
  const storageKey = "dmxforge-theme";
  const allowedThemes = new Set(["auto", "dark", "light"]);
  const themeLabels = {
    auto: "Auto",
    dark: "Dark",
    light: "Light",
  };
  const systemThemeQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;
  const customSelects = [];
  const checkboxSelects = [];
  let customSelectCounter = 0;
  const eventOptionPresets = {
    github: [
      { value: "push", label: "push" },
      { value: "pull_request", label: "pull_request" },
      { value: "issues", label: "issues" },
      { value: "issue_comment", label: "issue_comment" },
      { value: "release", label: "release" },
      { value: "create", label: "create" },
      { value: "delete", label: "delete" },
      { value: "workflow_run", label: "workflow_run" },
      { value: "ping", label: "ping" },
    ],
    gitlab: [
      { value: "push", label: "push" },
      { value: "tag_push", label: "tag_push" },
      { value: "merge_request", label: "merge_request" },
      { value: "pipeline", label: "pipeline" },
      { value: "release", label: "release" },
      { value: "note", label: "note" },
    ],
    gitea: [
      { value: "push", label: "push" },
      { value: "pull_request", label: "pull_request" },
      { value: "issues", label: "issues" },
      { value: "issue_comment", label: "issue_comment" },
      { value: "release", label: "release" },
      { value: "create", label: "create" },
      { value: "delete", label: "delete" },
      { value: "ping", label: "ping" },
    ],
  };

  function normalizeThemePreference(value) {
    return allowedThemes.has(value) ? value : "auto";
  }

  function resolveTheme(preference) {
    if (preference === "dark" || preference === "light") {
      return preference;
    }

    return systemThemeQuery?.matches ? "dark" : "light";
  }

  function readThemePreference() {
    try {
      return normalizeThemePreference(localStorage.getItem(storageKey));
    } catch (_) {
      return normalizeThemePreference(root.getAttribute("data-theme-preference"));
    }
  }

  function syncThemeControls(preference, resolvedTheme) {
    document.querySelectorAll("[data-theme-option]").forEach((button) => {
      const isActive = button.dataset.themeOption === preference;
      button.classList.toggle("is-active", isActive);
      button.setAttribute("aria-pressed", isActive ? "true" : "false");
    });

    const currentLabel =
      preference === "auto"
        ? `${themeLabels.auto} · ${themeLabels[resolvedTheme]}`
        : themeLabels[preference];

    document.querySelectorAll("[data-theme-current]").forEach((node) => {
      node.textContent = currentLabel;
    });
  }

  function applyThemePreference(value, options = {}) {
    const { persist = true } = options;
    const preference = normalizeThemePreference(value);
    const resolvedTheme = resolveTheme(preference);

    root.setAttribute("data-theme-preference", preference);
    root.setAttribute("data-theme", resolvedTheme);

    if (persist) {
      try {
        localStorage.setItem(storageKey, preference);
      } catch (_) {}
    }

    syncThemeControls(preference, resolvedTheme);
  }

  applyThemePreference(readThemePreference(), { persist: false });

  if (systemThemeQuery) {
    const handleSystemThemeChange = () => {
      if (normalizeThemePreference(root.getAttribute("data-theme-preference")) === "auto") {
        applyThemePreference("auto", { persist: false });
      }
    };

    if (typeof systemThemeQuery.addEventListener === "function") {
      systemThemeQuery.addEventListener("change", handleSystemThemeChange);
    } else if (typeof systemThemeQuery.addListener === "function") {
      systemThemeQuery.addListener(handleSystemThemeChange);
    }
  }

  document.addEventListener("click", (event) => {
    const themeOption = event.target.closest("[data-theme-option]");
    if (themeOption) {
      applyThemePreference(themeOption.dataset.themeOption);
    }
  });

  sidebarToggle?.addEventListener("click", () => {
    sidebar?.classList.toggle("is-open");
  });

  document.addEventListener("click", (e) => {
    const toggle = e.target.closest("[data-user-dropdown-toggle]");
    if (toggle) {
      const dropdown = toggle.closest(".user-dropdown");
      if (dropdown) {
        dropdown.classList.toggle("is-open");
        e.stopPropagation(); // Prevent immediate closing
      }
    }
  });

  function closeAllCustomSelects(except = null, returnFocus = false) {
    customSelects.forEach((instance) => {
      if (instance !== except) {
        instance.close(returnFocus);
      }
    });
  }

  function closeAllCheckboxSelects(except = null, returnFocus = false) {
    checkboxSelects.forEach((instance) => {
      if (instance !== except) {
        instance.close(returnFocus);
      }
    });
  }

  function splitListValue(value) {
    return String(value || "")
      .split(/[\n,]/)
      .map((item) => item.trim())
      .filter(Boolean);
  }

  function summarizeList(values, fallback) {
    if (!values.length) {
      return fallback;
    }

    if (values.length <= 2) {
      return values.join(", ");
    }

    return `${values.length} selected`;
  }

  function eventOptionsForProvider(provider, selectedValues) {
    const merged = new Map();

    (eventOptionPresets[provider] || eventOptionPresets.github).forEach((option) => {
      merged.set(option.value, option);
    });

    selectedValues.forEach((value) => {
      if (!merged.has(value)) {
        merged.set(value, { value, label: value });
      }
    });

    return Array.from(merged.values());
  }

  function mountFloatingMenu(instance, kind) {
    if (instance.menu.parentNode !== document.body) {
      document.body.appendChild(instance.menu);
    }

    instance.menu.classList.add("floating-select-menu", `floating-${kind}-menu`);
    instance.menu.style.display = "block";
    positionFloatingMenu(instance);
  }

  function unmountFloatingMenu(instance, kind) {
    instance.menu.classList.remove("floating-select-menu", `floating-${kind}-menu`);
    instance.menu.style.display = "";
    instance.menu.style.position = "";
    instance.menu.style.left = "";
    instance.menu.style.top = "";
    instance.menu.style.width = "";
    instance.menu.style.maxHeight = "";

    if (instance.menu.parentNode !== instance.menuHost) {
      instance.menuHost.appendChild(instance.menu);
    }
  }

  function positionFloatingMenu(instance) {
    if (!instance.wrapper.classList.contains("is-open")) {
      return;
    }

    const gap = 4;
    const margin = 12;
    const maxMenuHeight = 240;
    const rect = instance.trigger.getBoundingClientRect();
    const viewportHeight = window.innerHeight;
    const viewportWidth = window.innerWidth;
    const spaceBelow = viewportHeight - rect.bottom - margin;
    const spaceAbove = rect.top - margin;
    const preferUp = spaceBelow < 160 && spaceAbove > spaceBelow;
    const availableHeight = Math.max(
      120,
      Math.min(maxMenuHeight, preferUp ? spaceAbove - gap : spaceBelow - gap),
    );

    instance.menu.style.position = "fixed";
    const left = Math.max(margin, Math.min(rect.left, viewportWidth - rect.width - margin));
    instance.menu.style.left = `${left}px`;
    instance.menu.style.width = `${rect.width}px`;
    instance.menu.style.maxHeight = `${availableHeight}px`;

    const measuredHeight = Math.min(instance.menu.scrollHeight, availableHeight);
    const top = preferUp
      ? Math.max(margin, rect.top - measuredHeight - gap)
      : Math.min(viewportHeight - margin - measuredHeight, rect.bottom + gap);

    instance.menu.style.top = `${top}px`;
  }

  function syncOpenMenuPositions() {
    customSelects.forEach((instance) => {
      if (instance.wrapper.classList.contains("is-open")) {
        positionFloatingMenu(instance);
      }
    });

    checkboxSelects.forEach((instance) => {
      if (instance.wrapper.classList.contains("is-open")) {
        positionFloatingMenu(instance);
      }
    });
  }

  function createCustomSelect(select) {
    if (
      !(select instanceof HTMLSelectElement)
      || !select.classList.contains("select")
      || select.multiple
      || select.closest(".custom-select")
      || select.dataset.enhancedSelect === "false"
    ) {
      return null;
    }

    const wrapper = document.createElement("div");
    wrapper.className = "custom-select";
    select.parentNode.insertBefore(wrapper, select);
    wrapper.appendChild(select);

    select.classList.add("custom-select-native");
    select.tabIndex = -1;

    const trigger = document.createElement("button");
    trigger.type = "button";
    trigger.className = "custom-select-trigger";
    trigger.setAttribute("aria-haspopup", "listbox");
    trigger.setAttribute("aria-expanded", "false");

    const value = document.createElement("span");
    value.className = "custom-select-value";

    const indicator = document.createElement("span");
    indicator.className = "custom-select-indicator";
    indicator.setAttribute("aria-hidden", "true");

    trigger.append(value, indicator);

    const menu = document.createElement("div");
    menu.className = "custom-select-menu";
    menu.setAttribute("role", "listbox");
    menu.id = `custom-select-menu-${++customSelectCounter}`;
    trigger.setAttribute("aria-controls", menu.id);

    wrapper.append(trigger, menu);

    const instance = {
      wrapper,
      select,
      trigger,
      menu,
      menuHost: wrapper,
      options: [],
      close(returnFocus = false) {
        wrapper.classList.remove("is-open");
        trigger.setAttribute("aria-expanded", "false");
        unmountFloatingMenu(instance, "custom-select");

        if (returnFocus) {
          trigger.focus();
        }
      },
      open(focusIndex = -1) {
        if (select.disabled) {
          return;
        }

        closeAllCustomSelects(instance);
        wrapper.classList.add("is-open");
        trigger.setAttribute("aria-expanded", "true");
        mountFloatingMenu(instance, "custom-select");

        const selectedIndex = instance.getSelectedIndex();
        const resolvedIndex = focusIndex >= 0 ? focusIndex : selectedIndex;
        const option = instance.options[resolvedIndex] || instance.options.find((item) => !item.disabled);

        option?.focus();
      },
      getSelectedIndex() {
        return Array.from(select.options).findIndex((option) => option.selected);
      },
      focusAdjacent(index, direction) {
        if (!instance.options.length) {
          return;
        }

        let cursor = index;
        do {
          cursor += direction;
        } while (instance.options[cursor] && instance.options[cursor].disabled);

        instance.options[cursor]?.focus();
      },
      focusBoundary(direction) {
        const candidates = direction > 0 ? instance.options : [...instance.options].reverse();
        const option = candidates.find((item) => !item.disabled);
        option?.focus();
      },
      sync() {
        const selectedOption = select.selectedOptions[0] || select.options[0];
        value.textContent = selectedOption ? selectedOption.textContent.trim() : "";
        trigger.disabled = select.disabled;

        instance.options.forEach((optionButton, index) => {
          const option = select.options[index];
          const isSelected = Boolean(option?.selected);
          optionButton.classList.toggle("is-selected", isSelected);
          optionButton.setAttribute("aria-selected", isSelected ? "true" : "false");
          optionButton.disabled = Boolean(option?.disabled);
        });
      },
    };

    function rebuildOptions() {
      menu.innerHTML = "";
      instance.options = [];

      Array.from(select.options).forEach((option, index) => {
        const optionButton = document.createElement("button");
        optionButton.type = "button";
        optionButton.className = "custom-select-option";
        optionButton.setAttribute("role", "option");
        optionButton.textContent = option.textContent.trim();
        optionButton.disabled = option.disabled;

        optionButton.addEventListener("click", () => {
          if (option.disabled) {
            return;
          }

          select.value = option.value;
          select.dispatchEvent(new Event("change", { bubbles: true }));
          instance.close(true);
        });

        optionButton.addEventListener("keydown", (event) => {
          switch (event.key) {
            case "ArrowDown":
              event.preventDefault();
              instance.focusAdjacent(index, 1);
              break;
            case "ArrowUp":
              event.preventDefault();
              instance.focusAdjacent(index, -1);
              break;
            case "Home":
              event.preventDefault();
              instance.focusBoundary(1);
              break;
            case "End":
              event.preventDefault();
              instance.focusBoundary(-1);
              break;
            case "Escape":
              event.preventDefault();
              instance.close(true);
              break;
            case "Tab":
              instance.close(false);
              break;
            case " ": // Handled conditionally so we can type spaces in search
            case "Enter":
              event.preventDefault();
              optionButton.click();
              break;
            default:
              break;
          }
        });

        menu.appendChild(optionButton);
        instance.options.push(optionButton);
      });

      instance.sync();
      
      let searchInput = instance.menu.querySelector('.custom-select-search');
      if (!searchInput) {
        const searchContainer = document.createElement('div');
        searchContainer.className = 'custom-select-search-container';
        searchContainer.style.padding = '8px';
        searchContainer.style.borderBottom = '1px solid var(--border-color)';
        searchContainer.style.marginBottom = '4px';
        searchContainer.style.position = 'sticky';
        searchContainer.style.top = '0';
        searchContainer.style.background = 'var(--bg-secondary)';
        searchContainer.style.zIndex = '1';
        
        searchInput = document.createElement('input');
        searchInput.type = 'text';
        searchInput.className = 'input custom-select-search';
        searchInput.placeholder = 'Search...';
        searchInput.style.height = '28px';
        searchInput.style.fontSize = '12px';
        searchInput.style.marginBottom = '0';
        
        searchInput.addEventListener('input', (e) => {
          const query = e.target.value.toLowerCase();
          instance.options.forEach(btn => {
            const text = btn.textContent.toLowerCase();
            btn.style.display = text.includes(query) ? 'flex' : 'none';
          });
        });
        
        // Prevent click from bubbling up and closing menu
        searchContainer.addEventListener('click', (e) => e.stopPropagation());
        searchInput.addEventListener('keydown', (e) => {
            if(e.key === ' ') e.stopPropagation();
        });
        
        searchContainer.appendChild(searchInput);
        instance.menu.insertBefore(searchContainer, instance.menu.firstChild);
      }
    }

    trigger.addEventListener("click", () => {
      if (wrapper.classList.contains("is-open")) {
        instance.close(false);
        return;
      }

      instance.open();
    });

    trigger.addEventListener("keydown", (event) => {
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          instance.open(Math.max(instance.getSelectedIndex(), 0));
          break;
        case "ArrowUp":
          event.preventDefault();
          instance.open(instance.getSelectedIndex() > 0 ? instance.getSelectedIndex() : instance.options.length - 1);
          break;
        case " ":
        case "Enter":
          event.preventDefault();
          instance.open();
          break;
        default:
          break;
      }
    });

    select.addEventListener("change", () => instance.sync());
    select.form?.addEventListener("reset", () => {
      window.setTimeout(() => instance.sync(), 0);
    });

    rebuildOptions();
    return instance;
  }

  function createEventMultiSelect(wrapper) {
    const hiddenInput = wrapper.querySelector("[data-event-multiselect-input]");
    const trigger = wrapper.querySelector("[data-event-multiselect-trigger]");
    const value = wrapper.querySelector("[data-event-multiselect-value]");
    const menu = wrapper.querySelector("[data-event-multiselect-menu]");
    const optionsRoot = wrapper.querySelector("[data-event-multiselect-options]");
    const providerSelect = wrapper.closest("form")?.querySelector("select[name='provider']");

    if (!(hiddenInput instanceof HTMLInputElement) || !(trigger instanceof HTMLButtonElement) || !menu || !optionsRoot) {
      return null;
    }

    const instance = {
      wrapper,
      hiddenInput,
      trigger,
      menu,
      menuHost: wrapper,
      optionsRoot,
      providerSelect,
      selectedValues: new Set(splitListValue(hiddenInput.value)),
      close(returnFocus = false) {
        wrapper.classList.remove("is-open");
        trigger.setAttribute("aria-expanded", "false");
        unmountFloatingMenu(instance, "checkbox-select");

        if (returnFocus) {
          trigger.focus();
        }
      },
      open() {
        closeAllCustomSelects();
        closeAllCheckboxSelects(instance);
        wrapper.classList.add("is-open");
        trigger.setAttribute("aria-expanded", "true");
        mountFloatingMenu(instance, "checkbox-select");
      },
      syncValue() {
        const values = Array.from(instance.selectedValues);
        hiddenInput.value = values.join("\n");
        value.textContent = summarizeList(values, "All events");
      },
      render() {
        const selectedValues = Array.from(instance.selectedValues);
        const provider = providerSelect?.value || "github";
        const options = eventOptionsForProvider(provider, selectedValues);

        optionsRoot.replaceChildren();
        
        let searchInput = instance.menu.querySelector('.checkbox-select-search');
        if (!searchInput) {
          const searchContainer = document.createElement('div');
          searchContainer.className = 'checkbox-select-search-container';
          searchContainer.style.padding = '8px';
          searchContainer.style.borderBottom = '1px solid var(--border-color)';
          searchContainer.style.marginBottom = '4px';
          searchContainer.style.position = 'sticky';
          searchContainer.style.top = '0';
          searchContainer.style.background = 'var(--bg-secondary)';
          searchContainer.style.zIndex = '1';
          
          searchInput = document.createElement('input');
          searchInput.type = 'text';
          searchInput.className = 'input checkbox-select-search';
          searchInput.placeholder = 'Search...';
          searchInput.style.height = '28px';
          searchInput.style.fontSize = '12px';
          searchInput.style.marginBottom = '0';
          
          searchInput.addEventListener('input', (e) => {
            const query = e.target.value.toLowerCase();
            const labels = optionsRoot.querySelectorAll('.checkbox-select-option');
            labels.forEach(label => {
              const text = label.textContent.toLowerCase();
              label.style.display = text.includes(query) ? 'flex' : 'none';
            });
          });
          
          searchContainer.appendChild(searchInput);
          instance.menu.insertBefore(searchContainer, optionsRoot);
        }

        options.forEach((option) => {
          const label = document.createElement("label");
          label.className = "checkbox-select-option";

          const input = document.createElement("input");
          input.type = "checkbox";
          input.checked = instance.selectedValues.has(option.value);
          input.value = option.value;

          input.addEventListener("change", () => {
            if (input.checked) {
              instance.selectedValues.add(option.value);
            } else {
              instance.selectedValues.delete(option.value);
            }

            instance.syncValue();
          });

          const text = document.createElement("span");
          text.textContent = option.label;

          label.append(input, text);
          optionsRoot.appendChild(label);
        });

        instance.syncValue();
      },
      reset() {
        instance.selectedValues = new Set(splitListValue(hiddenInput.defaultValue));
        instance.render();
      },
    };

    trigger.addEventListener("click", () => {
      if (wrapper.classList.contains("is-open")) {
        instance.close(false);
      } else {
        instance.open();
      }
    });

    providerSelect?.addEventListener("change", () => instance.render());
    hiddenInput.form?.addEventListener("reset", () => {
      window.setTimeout(() => instance.reset(), 0);
    });

    instance.render();
    return instance;
  }

  document.querySelectorAll("select").forEach((select) => {
    const instance = createCustomSelect(select);

    if (instance) {
      customSelects.push(instance);
    }
  });

  document.querySelectorAll("[data-event-multiselect]").forEach((wrapper) => {
    const instance = createEventMultiSelect(wrapper);

    if (instance) {
      checkboxSelects.push(instance);
    }
  });

  window.addEventListener("resize", syncOpenMenuPositions);
  document.addEventListener("scroll", syncOpenMenuPositions, true);

  document.addEventListener("click", (event) => {
    if (!event.target.closest(".custom-select") && !event.target.closest(".floating-custom-select-menu")) {
      closeAllCustomSelects();
    }

    if (!event.target.closest(".checkbox-select") && !event.target.closest(".floating-checkbox-select-menu")) {
      closeAllCheckboxSelects();
    }
    
    if (!event.target.closest(".user-dropdown")) {
      document.querySelectorAll(".user-dropdown.is-open").forEach(el => el.classList.remove("is-open"));
    }
  });

  const closeModal = (backdrop) => {
    body.classList.remove("has-modal");
    document.body.style.overflow = ""; // Restore page scroll
    const closeUrl = backdrop?.dataset.closeUrl || window.location.pathname;
    window.location.assign(closeUrl);
  };

  document.querySelectorAll("[data-modal-backdrop]").forEach((backdrop) => {
    if (backdrop.classList.contains("is-open")) {
      body.classList.add("has-modal");
      document.body.style.overflow = "hidden"; // Block page scroll
    }

    backdrop.addEventListener("click", (event) => {
      if (event.target === backdrop) {
        return;
      }

      if (event.target.closest("[data-close-modal]")) {
        event.preventDefault();
        closeModal(backdrop);
      }
    });
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") {
      return;
    }

    const openSelect = customSelects.find((instance) => instance.wrapper.classList.contains("is-open"));
    if (openSelect) {
      event.preventDefault();
      openSelect.close(true);
      return;
    }

    const openCheckboxSelect = checkboxSelects.find((instance) => instance.wrapper.classList.contains("is-open"));
    if (openCheckboxSelect) {
      event.preventDefault();
      openCheckboxSelect.close(true);
      return;
    }

    const openBackdrop = document.querySelector("[data-modal-backdrop].is-open");
    if (openBackdrop) {
      return;
    }
  });

  document.querySelectorAll("[data-copy-text]").forEach((button) => {
    const originalLabel = button.textContent;

    button.addEventListener("click", async () => {
      const value = button.dataset.copyText || "";

      try {
        await navigator.clipboard.writeText(value);
        button.textContent = "Copied";
        window.setTimeout(() => {
          button.textContent = originalLabel;
        }, 1200);
      } catch (_) {
        button.textContent = "Failed";
        window.setTimeout(() => {
          button.textContent = originalLabel;
        }, 1200);
      }
    });
  });

  document.querySelectorAll("[data-toast]").forEach((toast) => {
    window.requestAnimationFrame(() => toast.classList.add("is-visible"));
    window.setTimeout(() => toast.classList.remove("is-visible"), 4200);
  });

  function parseBoolean(value) {
    return value === true || value === "true" || value === "1";
  }

  function cleanText(value, fallback = "") {
    return typeof value === "string" && value.trim() ? value.trim() : fallback;
  }

  function normalizeColor(value, fallback = "#FF7000") {
    return /^#[\dA-F]{6}$/i.test(cleanText(value)) ? cleanText(value).toUpperCase() : fallback;
  }

  function initials(value) {
    const tokens = cleanText(value, "DM")
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2);

    return (tokens.map((token) => token[0]).join("") || "DM").toUpperCase();
  }

  function selectedOptionLabel(select, fallback = "") {
    const option = select?.selectedOptions?.[0];
    return cleanText(option?.dataset?.label || option?.textContent || "", fallback);
  }

  function setVisibility(node, visible) {
    if (node) {
      node.hidden = !visible;
    }
  }

  function fillList(list, items) {
    if (!list) {
      return;
    }

    list.replaceChildren();

    items.forEach((item) => {
      const entry = document.createElement("li");
      entry.textContent = item;
      list.appendChild(entry);
    });
  }

  function fillPills(container, items) {
    if (!container) {
      return;
    }

    container.replaceChildren();

    items.forEach((item) => {
      const pill = document.createElement("span");
      pill.className = "route-filter-pill";
      pill.textContent = item;
      container.appendChild(pill);
    });
  }

  function paintDiscordPreview(preview, model) {
    if (!preview) {
      return;
    }

    const avatar = preview.querySelector("[data-discord-preview-avatar]");
    const username = preview.querySelector("[data-discord-preview-username]");
    const timestamp = preview.querySelector("[data-discord-preview-timestamp]");
    const card = preview.querySelector("[data-discord-preview-card]");
    const title = preview.querySelector("[data-discord-preview-title]");
    const description = preview.querySelector("[data-discord-preview-description]");
    const repo = preview.querySelector("[data-discord-preview-repo]");
    const branch = preview.querySelector("[data-discord-preview-branch]");
    const status = preview.querySelector("[data-discord-preview-status]");
    const commits = preview.querySelector("[data-discord-preview-commits]");
    const footerRow = preview.querySelector("[data-discord-preview-footer-row]");
    const footer = preview.querySelector("[data-discord-preview-footer]");

    if (card) {
      card.style.setProperty("--discord-embed-color", normalizeColor(model.accentColor));
    }

    if (avatar) {
      const showAvatar = Boolean(model.showAvatar);
      avatar.classList.toggle("is-hidden", !showAvatar);
      avatar.textContent = showAvatar ? initials(model.username) : "";
      avatar.style.backgroundImage = model.avatarUrl ? `url("${model.avatarUrl.replace(/"/g, "%22")}")` : "";
    }

    if (username) {
      username.textContent = cleanText(model.username, "dmxforge");
    }

    if (timestamp) {
      timestamp.textContent = cleanText(model.timestamp, "Today at 16:42");
      setVisibility(timestamp, Boolean(model.showTimestamp));
    }

    if (title) {
      title.textContent = cleanText(model.title, "Discord preview");
    }

    if (description) {
      description.textContent = cleanText(model.description, "Select a template to preview the embed.");
    }

    if (repo) {
      repo.textContent = `repo: ${cleanText(model.repo, "acme/dmxforge")}`;
      setVisibility(repo, Boolean(model.showRepoLink));
    }

    if (branch) {
      branch.textContent = `branch: ${cleanText(model.branch, "main")}`;
      setVisibility(branch, Boolean(model.showBranch));
    }

    if (status) {
      status.textContent = `${cleanText(model.statusLabel, "status")}: ${cleanText(model.status, "success")}`;
      setVisibility(status, Boolean(model.showStatusBadge));
    }

    fillList(commits, Boolean(model.showCommits) ? model.commits : []);

    const footerParts = [];
    if (cleanText(model.footer)) {
      footerParts.push(cleanText(model.footer));
    }
    if (Boolean(model.showTimestamp)) {
      footerParts.push(cleanText(model.timestamp, "Today at 16:42"));
    }

    if (footer) {
      footer.textContent = footerParts.join(" • ");
    }
    setVisibility(footerRow, footerParts.length > 0);
  }

  function syncTemplatePreview(form, outputText) {
    const scope = form.closest("[data-preview-scope]") || form.closest(".panel") || form;
    const preview = scope.querySelector("[data-discord-preview='template']");

    if (!preview) {
      return;
    }

    paintDiscordPreview(preview, {
      username: cleanText(form.querySelector("input[name='username_override']")?.value, "dmxforge"),
      avatarUrl: cleanText(form.querySelector("input[name='avatar_url_override']")?.value),
      showAvatar: form.querySelector("input[name='show_avatar']")?.checked ?? true,
      accentColor: cleanText(form.querySelector("input[name='embed_color']")?.value, "#FF7000"),
      title: cleanText(form.querySelector("input[name='name']")?.value, "Custom template"),
      description: outputText,
      repo: "acme/dmxforge",
      showRepoLink: form.querySelector("input[name='show_repo_link']")?.checked ?? true,
      branch: "main",
      showBranch: form.querySelector("input[name='show_branch']")?.checked ?? true,
      status: cleanText(form.querySelector("select[name='format_style']")?.value, "success"),
      statusLabel: "style",
      showStatusBadge: form.querySelector("input[name='show_status_badge']")?.checked ?? true,
      commits: [
        "abc1234 Bootstrap dashboard UI",
        "bcd2345 Refine Discord embed preview",
        "cde3456 Route webhook to Discord",
      ],
      showCommits: form.querySelector("input[name='show_commits']")?.checked ?? true,
      footer: cleanText(form.querySelector("input[name='footer_text']")?.value),
      timestamp: "Today at 16:42",
      showTimestamp: form.querySelector("input[name='show_timestamp']")?.checked ?? true,
    });
  }

  function syncRoutePreview(form) {
    const preview = form.querySelector("[data-discord-preview='route']");
    if (!preview) {
      return;
    }

    const sourceSelect = form.querySelector("select[name='source_id']");
    const destinationSelect = form.querySelector("select[name='destination_id']");
    const templateSelect = form.querySelector("select[name='template_id']");
    const providerSelect = form.querySelector("select[name='provider_filter']");
    const eventTypeInput = form.querySelector("input[name='event_type_filter']");
    const branchInput = form.querySelector("input[name='branch_prefix_filter']");
    const repositoryInput = form.querySelector("input[name='repository_filter']");
    const skipInput = form.querySelector("input[name='skip_keyword']");

    const sourceLabel = selectedOptionLabel(sourceSelect, "All sources");
    const destinationLabel = selectedOptionLabel(destinationSelect, "Destination");
    const templateOption = templateSelect?.selectedOptions?.[0];
    const templateLabel = cleanText(templateOption?.dataset?.label, "Template");

    const filterSummary = [
      providerSelect?.value ? `provider: ${selectedOptionLabel(providerSelect, providerSelect.value)}` : "",
      cleanText(eventTypeInput?.value) ? `event: ${cleanText(eventTypeInput.value)}` : "",
      cleanText(branchInput?.value) ? `branch: ${cleanText(branchInput.value)}` : "",
      cleanText(repositoryInput?.value) ? `repo: ${cleanText(repositoryInput.value)}` : "",
      cleanText(skipInput?.value) ? `skip: ${cleanText(skipInput.value)}` : "",
    ].filter(Boolean);

    const sourceNode = form.querySelector("[data-route-preview-source]");
    const templateNode = form.querySelector("[data-route-preview-template]");
    const destinationNode = form.querySelector("[data-route-preview-destination]");

    if (sourceNode) {
      sourceNode.textContent = sourceLabel;
    }
    if (templateNode) {
      templateNode.textContent = templateLabel;
    }
    if (destinationNode) {
      destinationNode.textContent = destinationLabel;
    }

    fillPills(
      form.querySelector("[data-route-preview-filters]"),
      filterSummary.length ? filterSummary : ["No extra filters"],
    );

    paintDiscordPreview(preview, {
      username: cleanText(templateOption?.dataset?.usernameOverride, "dmxforge"),
      avatarUrl: cleanText(templateOption?.dataset?.avatarUrlOverride),
      showAvatar: parseBoolean(templateOption?.dataset?.showAvatar),
      accentColor: cleanText(templateOption?.dataset?.embedColor, "#FF7000"),
      title: templateLabel,
      description: cleanText(
        templateOption?.dataset?.previewOutput,
        "Select a template to preview the embed.",
      ),
      repo: cleanText(repositoryInput?.value, "acme/dmxforge"),
      showRepoLink: parseBoolean(templateOption?.dataset?.showRepoLink),
      branch: cleanText(branchInput?.value, "main"),
      showBranch: parseBoolean(templateOption?.dataset?.showBranch),
      status: cleanText(eventTypeInput?.value, providerSelect?.value || "success"),
      statusLabel: cleanText(eventTypeInput?.value) ? "event" : "status",
      showStatusBadge: parseBoolean(templateOption?.dataset?.showStatusBadge),
      commits: [
        "abc1234 Bootstrap dashboard UI",
        "bcd2345 Refine Discord embed preview",
      ],
      showCommits: parseBoolean(templateOption?.dataset?.showCommits),
      footer: cleanText(templateOption?.dataset?.footerText, destinationLabel),
      timestamp: "Today at 16:42",
      showTimestamp: parseBoolean(templateOption?.dataset?.showTimestamp),
    });

  }

  document.querySelectorAll("[data-preview-form]").forEach((form) => {
    const scope = form.closest("[data-preview-scope]") || form.closest(".panel") || form;
    const output = scope.querySelector("[data-preview-output]");
    const textarea = form.querySelector("textarea[name='body_template'], textarea[name='template'], textarea");
    let debounceId = null;
    const previewOnlyForm = !cleanText(form.getAttribute("action"));

    if (!output || !textarea) {
      return;
    }

    async function renderPreview() {
      output.innerHTML = '<div class="skeleton" style="width: 80%"></div><div class="skeleton" style="width: 60%"></div><div class="skeleton" style="width: 90%"></div>';
      syncTemplatePreview(form, "Rendering...");

      try {
        const response = await fetch("/api/preview", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ template: textarea.value }),
        });

        const data = await response.json();
        const resultText = response.ok ? data.rendered : data.error || "Preview failed";
        
        output.textContent = "";
        output.classList.add("response-block");
        let i = 0;
        
        function streamTokens() {
          if (i < resultText.length) {
            // Add next character
            const textPart = resultText.substring(0, i + 1);
            output.innerHTML = textPart.replace(/</g, "&lt;").replace(/>/g, "&gt;") + '<span class="streaming-cursor"></span>';
            i++;
            // Faster streaming speed
            setTimeout(streamTokens, 5);
          } else {
            output.innerHTML = resultText.replace(/</g, "&lt;").replace(/>/g, "&gt;");
            syncTemplatePreview(form, resultText);
          }
        }
        streamTokens();

      } catch (_) {
        output.textContent = "Preview failed";
      }
    }

    if (previewOnlyForm) {
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        renderPreview();
      });
    }

    textarea.addEventListener("input", () => {
      clearTimeout(debounceId);
      debounceId = window.setTimeout(renderPreview, 300);
    });

    form.addEventListener("input", () => syncTemplatePreview(form, output.textContent));
    form.addEventListener("change", () => syncTemplatePreview(form, output.textContent));
    syncTemplatePreview(form, output.textContent);
  });

  document.querySelectorAll("[data-route-preview-form]").forEach((form) => {
    const sync = () => syncRoutePreview(form);
    form.addEventListener("input", sync);
    form.addEventListener("change", sync);
    sync();
  });
})();


// UX Enhancement: Auto-expand textareas
document.addEventListener('DOMContentLoaded', () => {
    const textareas = document.querySelectorAll('textarea');
    textareas.forEach(textarea => {
        textarea.addEventListener('input', function() {
            this.style.height = 'auto';
            this.style.height = (this.scrollHeight) + 'px';
        });
        // Trigger once on load
        if(textarea.value) {
            textarea.style.height = 'auto';
            textarea.style.height = (textarea.scrollHeight) + 'px';
        }
    });
});
