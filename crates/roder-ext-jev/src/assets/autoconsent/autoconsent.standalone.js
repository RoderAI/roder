"use strict";
(() => {
  var __typeError = (msg) => {
    throw TypeError(msg);
  };
  var __accessCheck = (obj, member, msg) => member.has(obj) || __typeError("Cannot " + msg);
  var __privateGet = (obj, member, getter) => (__accessCheck(obj, member, "read from private field"), getter ? getter.call(obj) : member.get(obj));
  var __privateAdd = (obj, member, value) => member.has(obj) ? __typeError("Cannot add the same private member more than once") : member instanceof WeakSet ? member.add(obj) : member.set(obj, value);
  var __privateSet = (obj, member, value, setter) => (__accessCheck(obj, member, "write to private field"), setter ? setter.call(obj, value) : member.set(obj, value), value);

  // lib/rules.ts
  var SUPPORTED_RULE_STEP_VERSION = 3;

  // lib/random.ts
  function getRandomID() {
    if (crypto && typeof crypto.randomUUID !== "undefined") {
      return crypto.randomUUID();
    }
    return Math.random().toString();
  }

  // lib/eval-handler.ts
  var Deferred = class {
    constructor(id, timeout = 1e3) {
      this.id = id;
      this.promise = new Promise((resolve, reject) => {
        this.resolve = resolve;
        this.reject = reject;
      });
      this.timer = window.setTimeout(() => {
        this.reject(new Error("timeout"));
      }, timeout);
    }
  };
  var evalState = {
    pending: /* @__PURE__ */ new Map(),
    sendContentMessage: null
  };
  function requestEval(code, snippetId) {
    if (!evalState.sendContentMessage) {
      return Promise.reject(new Error("AutoConsent is not initialized yet"));
    }
    const id = getRandomID();
    evalState.sendContentMessage({
      type: "eval",
      id,
      code,
      snippetId
    });
    const deferred = new Deferred(id);
    evalState.pending.set(deferred.id, deferred);
    return deferred.promise;
  }
  function resolveEval(id, value) {
    const deferred = evalState.pending.get(id);
    if (deferred) {
      evalState.pending.delete(id);
      deferred.timer && window.clearTimeout(deferred.timer);
      deferred.resolve(value);
    } else {
      console.warn("no eval #", id);
    }
  }

  // lib/utils.ts
  function getStyleElement(styleOverrideElementId = "autoconsent-css-rules") {
    const styleSelector = `style#${styleOverrideElementId}`;
    const existingElement = document.querySelector(styleSelector);
    if (existingElement && existingElement instanceof HTMLStyleElement) {
      return existingElement;
    } else {
      const parent = document.head || document.getElementsByTagName("head")[0] || document.documentElement;
      const css = document.createElement("style");
      css.id = styleOverrideElementId;
      parent.appendChild(css);
      return css;
    }
  }
  function getHidingStyle(method) {
    const hidingSnippet = method === "opacity" ? `opacity: 0` : `display: none`;
    return `${hidingSnippet} !important; z-index: -1 !important; pointer-events: none !important;`;
  }
  function hideElements(styleEl, selector, method = "display") {
    const rule = `${selector} { ${getHidingStyle(method)} } `;
    if (styleEl instanceof HTMLStyleElement) {
      styleEl.innerText += rule;
      return selector.length > 0;
    }
    return false;
  }
  function appendStylesheetRule(styleEl, cssRule, marker) {
    if (!(styleEl instanceof HTMLStyleElement) || cssRule.length === 0) {
      return false;
    }
    const markerComment = marker ? `/* autoconsent:${marker} */` : "";
    if (markerComment && styleEl.innerText.includes(markerComment)) {
      return true;
    }
    if (!markerComment && styleEl.innerText.includes(cssRule.trim())) {
      return true;
    }
    styleEl.innerText += `${markerComment}${markerComment ? " " : ""}${cssRule} `;
    return true;
  }
  async function waitFor(predicate, maxTimes, interval) {
    const result = await predicate();
    if (!result && maxTimes > 0) {
      return new Promise((resolve) => {
        setTimeout(async () => {
          resolve(waitFor(predicate, maxTimes - 1, interval));
        }, interval);
      });
    }
    return Promise.resolve(result);
  }
  function isElementVisible(elem) {
    if (!elem) {
      return false;
    }
    if (elem.offsetParent !== null) {
      return true;
    } else {
      const css = window.getComputedStyle(elem);
      if (css.position === "fixed" && css.display !== "none") {
        return true;
      }
    }
    return false;
  }
  function copyObject(data) {
    if (globalThis.structuredClone) {
      return structuredClone(data);
    }
    return JSON.parse(JSON.stringify(data));
  }
  function normalizeConfig(providedConfig) {
    const defaultConfig = {
      enabled: true,
      autoAction: "optOut",
      // if falsy, the extension will wait for an explicit user signal before opting in/out
      disabledCmps: [],
      enablePrehide: true,
      enableCosmeticRules: true,
      enableGeneratedRules: true,
      enableHeuristicDetection: false,
      enablePopupMutationObserver: false,
      detectRetries: 20,
      isMainWorld: false,
      prehideTimeout: 2e3,
      visualTest: false,
      logs: {
        lifecycle: false,
        rulesteps: false,
        detectionsteps: false,
        evals: false,
        errors: true,
        messages: false,
        waits: false
      },
      performanceLoggingEnabled: false,
      heuristicPopupSearchTimeout: 100,
      heuristicMode: "off"
      // heuristic disabled by default
    };
    const updatedConfig = copyObject(defaultConfig);
    for (const key of Object.keys(defaultConfig)) {
      if (typeof providedConfig[key] !== "undefined") {
        updatedConfig[key] = providedConfig[key];
      }
    }
    return updatedConfig;
  }
  function scheduleWhenIdle(callback, timeout = 500) {
    if (globalThis.requestIdleCallback) {
      requestIdleCallback(callback, { timeout });
    } else {
      setTimeout(callback, 0);
    }
  }
  function highlightNode(node) {
    const highlightedNode = node;
    if (!node.style) return;
    if (highlightedNode.__oldStyles !== void 0) {
      return;
    }
    if (node.hasAttribute("style")) {
      highlightedNode.__oldStyles = node.style.cssText;
    }
    node.style.animation = "pulsate .5s infinite";
    node.style.outline = "solid red";
    let styleTag = document.querySelector("style#autoconsent-debug-styles");
    if (!styleTag) {
      styleTag = document.createElement("style");
      styleTag.id = "autoconsent-debug-styles";
    }
    styleTag.textContent = `
      @keyframes pulsate {
        0% {
          outline-width: 8px;
          outline-offset: -4px;
        }
        50% {
          outline-width: 4px;
          outline-offset: -2px;
        }
        100% {
          outline-width: 8px;
          outline-offset: -4px;
        }
      }
    `;
    document.head.appendChild(styleTag);
  }
  function unhighlightNode(node) {
    const highlightedNode = node;
    if (!node.style || !node.hasAttribute("style")) return;
    if (highlightedNode.__oldStyles !== void 0) {
      node.style.cssText = highlightedNode.__oldStyles;
      delete highlightedNode.__oldStyles;
    } else {
      node.removeAttribute("style");
    }
  }
  function isTopFrame() {
    return window.top === window && (!globalThis.location.ancestorOrigins || globalThis.location.ancestorOrigins.length === 0);
  }

  // lib/eval-snippets.ts
  var snippets = {
    // code-based rules
    EVAL_0: () => console.log(1),
    EVAL_DIDOMI_OPT_OUT: () => {
      if (window.Didomi) {
        window.Didomi.setUserDisagreeToAll();
        return true;
      }
      return false;
    },
    EVAL_DIDOMI_TEST: () => {
      const purposes = window.Didomi?.getCurrentUserStatus?.()?.purposes;
      if (purposes) {
        return Object.values(purposes).some((p) => !p.enabled);
      }
      const disabled = window.Didomi?.getUserConsentStatusForAll?.()?.purposes?.disabled;
      return Array.isArray(disabled) && disabled.length > 0;
    },
    EVAL_CONSENTMANAGER_1: () => {
      const cmpData = window.__cmp?.("getCMPData");
      return !!cmpData && typeof cmpData === "object";
    },
    EVAL_CONSENTMANAGER_2: () => window.__cmp?.("consentStatus")?.userChoiceExists === false,
    EVAL_CONSENTMANAGER_3: () => __cmp("setConsent", 0),
    EVAL_CONSENTMANAGER_4: () => __cmp("setConsent", 1),
    EVAL_CONSENTMANAGER_5: () => __cmp("consentStatus").userChoiceExists,
    EVAL_CONSENTMANAGER_NCMP_REJECT: () => {
      if (typeof window.__npcmp !== "function") return false;
      window.__npcmp("reject");
      return true;
    },
    EVAL_CONSENTMANAGER_NCMP_ACCEPT: () => {
      if (typeof window.__npcmp !== "function") return false;
      window.__npcmp("save");
      return true;
    },
    EVAL_COOKIEBOT_1: () => !!window.Cookiebot,
    EVAL_COOKIEBOT_2: () => !window.Cookiebot.hasResponse && window.Cookiebot.dialog?.visible === true,
    EVAL_COOKIEBOT_3: () => window.Cookiebot.withdraw() || true,
    EVAL_COOKIEBOT_4: () => window.Cookiebot.hide() || true,
    EVAL_COOKIEBOT_5: () => window.Cookiebot.declined === true,
    EVAL_KLARO_1: () => {
      const config = globalThis.klaroConfig || globalThis.klaro?.getManager && globalThis.klaro.getManager().config;
      if (!config) {
        return true;
      }
      const optionalServices = (config.services || config.apps).filter((s) => !s.required).map((s) => s.name);
      if (klaro && klaro.getManager) {
        const manager = klaro.getManager();
        return optionalServices.every((name) => !manager.consents[name]);
      } else if (klaroConfig && klaroConfig.storageMethod === "cookie") {
        const cookieName = klaroConfig.cookieName || klaroConfig.storageName;
        const consents = JSON.parse(
          decodeURIComponent(
            document.cookie.split(";").find((c) => c.trim().startsWith(cookieName)).split("=")[1]
          )
        );
        return Object.keys(consents).filter((k) => optionalServices.includes(k)).every((k) => consents[k] === false);
      }
    },
    EVAL_KLARO_OPEN_POPUP: () => {
      klaro.show(void 0, true);
    },
    EVAL_KLARO_TRY_API_OPT_OUT: () => {
      if (window.klaro && typeof klaro.show === "function" && typeof klaro.getManager === "function") {
        try {
          klaro.getManager().changeAll(false);
          klaro.getManager().saveAndApplyConsents();
          return true;
        } catch (e) {
          console.warn(e);
          return false;
        }
      }
      return false;
    },
    EVAL_ONETRUST_1: () => window.OnetrustActiveGroups.split(",").filter((s) => s.length > 0).length <= 1,
    EVAL_TRUSTARC_TOP: () => window && window.truste && window.truste.eu.bindMap.prefCookie === "0",
    EVAL_TRUSTARC_FRAME_TEST: () => window && window.QueryString && window.QueryString.preferences === "0",
    EVAL_TRUSTARC_FRAME_GTM: () => window && window.QueryString && window.QueryString.gtm === "1",
    // declarative rules
    EVAL_ADOPT_TEST: () => !!localStorage.getItem("adoptConsentMode"),
    EVAL_ADULTFRIENDFINDER_TEST: () => !!localStorage.getItem("cookieConsent"),
    EVAL_AYLO_COOKIE_MANAGER_READY: () => !!(window.wl_cookie_consent_manager || window._Cookie_Consent_Manager_brand),
    EVAL_BAHN_TEST: () => utag.gdpr.getSelectedCategories().length === 1,
    EVAL_BIGCOMMERCE_CONSENT_MANAGER_DETECT: () => !!(window.consentManager && window.consentManager.version),
    EVAL_BORLABS_0: () => !JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((c) => c.indexOf("borlabs-cookie") !== -1).split("=", 2)[1]
      )
    ).consents.statistics,
    EVAL_CC_BANNER2_0: () => !!document.cookie.match(/sncc=[^;]+D%3Dtrue/),
    EVAL_COINBASE_0: () => JSON.parse(decodeURIComponent(document.cookie.match(/cm_(eu|default)_preferences=([0-9a-zA-Z\\{\\}\\[\\]%:]*);?/)[2])).consent.length <= 1,
    EVAL_COOKIE_LAW_INFO_0: () => {
      if (CLI.disableAllCookies) CLI.disableAllCookies();
      if (CLI.reject_close) CLI.reject_close();
      document.body.classList.remove("cli-barmodal-open");
      return true;
    },
    EVAL_COOKIE_LAW_INFO_DETECT: () => !!window.CLI,
    EVAL_COOKIE_MANAGER_POPUP_0: () => JSON.parse(
      document.cookie.split(";").find((c) => c.trim().startsWith("CookieLevel")).split("=")[1]
    ).social === false,
    EVAL_COOKIEALERT_0: () => document.querySelector("body").removeAttribute("style") || true,
    EVAL_COOKIEALERT_1: () => document.querySelector("body").removeAttribute("style") || true,
    EVAL_COOKIEALERT_2: () => window.CookieConsent.declined === true,
    EVAL_COOKIEFIRST_0: () => ((o) => o.performance === false && o.functional === false && o.advertising === false)(
      JSON.parse(
        decodeURIComponent(
          document.cookie.split(";").find((c) => c.indexOf("cookiefirst") !== -1).trim()
        ).split("=")[1]
      )
    ),
    EVAL_COOKIEFIRST_1: () => document.querySelectorAll("button[data-cookiefirst-accent-color=true][role=checkbox]:not([disabled])").forEach((i) => i.getAttribute("aria-checked") === "true" && i.click()) || true,
    EVAL_COOKIEINFORMATION_0: () => CookieInformation.declineAllCategories() || true,
    EVAL_COOKIEINFORMATION_1: () => CookieInformation.submitAllCategories() || true,
    EVAL_ETSY_0: () => document.querySelectorAll(".gdpr-overlay-body input").forEach((toggle) => {
      toggle.checked = false;
    }) || true,
    EVAL_ETSY_1: () => document.querySelector(".gdpr-overlay-view button[data-wt-overlay-close]").click() || true,
    EVAL_EZOIC_0: () => ezCMP.handleAcceptAllClick(),
    EVAL_FIDES_DETECT_POPUP: () => window.Fides?.initialized,
    EVAL_GDPR_LEGAL_COOKIE_DETECT_CMP: () => !!window.GDPR_LC,
    EVAL_GDPR_LEGAL_COOKIE_TEST: () => !!window.GDPR_LC?.userConsentSetting,
    EVAL_IUBENDA_0: () => document.querySelectorAll(".purposes-item input[type=checkbox]:not([disabled])").forEach((x) => {
      if (x.checked) x.click();
    }) || true,
    EVAL_IUBENDA_1: () => !!document.cookie.match(/_iub_cs-\d+=/),
    EVAL_KROWN_COOKIE_BANNER_TEST: () => localStorage.getItem("krown-cookie-banner") === "true",
    EVAL_MICROSOFT_0: () => Array.from(document.querySelectorAll("div > button")).filter((el) => el.innerText.match("Reject|Ablehnen"))[0].click() || true,
    EVAL_MICROSOFT_1: () => Array.from(document.querySelectorAll("div > button")).filter((el) => el.innerText.match("Accept|Annehmen"))[0].click() || true,
    EVAL_MICROSOFT_2: () => !!document.cookie.match("MSCC|GHCC"),
    EVAL_MOOVE_0: () => document.querySelectorAll("#moove_gdpr_cookie_modal input").forEach((i) => {
      if (!i.disabled) i.checked = i.name === "moove_gdpr_strict_cookies" || i.id === "moove_gdpr_strict_cookies";
    }) || true,
    EVAL_NHNIEUWS_TEST: () => !!localStorage.getItem("psh:cookies-seen"),
    EVAL_OSANO_DETECT: () => !!window.Osano?.cm?.dialogOpen,
    EVAL_PANDECTES_TEST: () => document.cookie.includes("_pandectes_gdpr=") && JSON.parse(
      atob(
        document.cookie.split(";").find((s) => s.trim().startsWith("_pandectes_gdpr")).split("=")[1]
      )
    ).status === "deny",
    EVAL_POVR_GOBACK: () => window.history.back() || true,
    EVAL_PUBTECH_0: () => document.cookie.includes("euconsent-v2") && (document.cookie.match(/.YAAAAAAAAAAA/) || document.cookie.match(/.aAAAAAAAAAAA/) || document.cookie.match(/.YAAACFgAAAAA/)),
    EVAL_SHOPIFY_TEST: () => document.cookie.includes("gdpr_cookie_consent=0") || document.cookie.includes("_tracking_consent=") && JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((s) => s.trim().startsWith("_tracking_consent")).split("=")[1]
      )
    ).purposes.a === false,
    EVAL_SKYSCANNER_TEST: () => document.cookie.match(/gdpr=[^;]*adverts:::false/) && !document.cookie.match(/gdpr=[^;]*init:::true/),
    EVAL_SIRDATA_UNBLOCK_SCROLL: () => {
      document.documentElement.classList.forEach((cls) => {
        if (cls.startsWith("sd-cmp-")) document.documentElement.classList.remove(cls);
      });
      return true;
    },
    EVAL_STEAMPOWERED_0: () => JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((s) => s.trim().startsWith("cookieSettings")).split("=")[1]
      )
    ).preference_state === 2,
    EVAL_TAKEALOT_0: () => document.body.classList.remove("freeze") || (document.body.style = "") || true,
    EVAL_TARTEAUCITRON_0: () => tarteaucitron.userInterface.respondAll(false) || true,
    EVAL_TARTEAUCITRON_1: () => tarteaucitron.userInterface.respondAll(true) || true,
    EVAL_TARTEAUCITRON_2: () => document.cookie.match(/tarteaucitron=[^;]*/)?.[0].includes("false"),
    EVAL_TEALIUM_0: () => typeof window.utag !== "undefined" && typeof utag.gdpr === "object",
    EVAL_TEALIUM_1: () => utag.gdpr.setConsentValue(false) || true,
    EVAL_TEALIUM_DONOTSELL: () => utag.gdpr.dns?.setDnsState(false) || true,
    EVAL_TEALIUM_2: () => utag.gdpr.setConsentValue(true) || true,
    EVAL_TEALIUM_3: () => utag.gdpr.getConsentState() !== 1,
    EVAL_TEALIUM_DONOTSELL_CHECK: () => utag.gdpr.dns?.getDnsState() !== 1,
    EVAL_TESTCMP_STEP: () => !!document.querySelector("#reject-all"),
    EVAL_TESTCMP_0: () => window.results.results[0] === "button_clicked",
    EVAL_TESTCMP_COSMETIC_0: () => window.results.results[0] === "banner_hidden",
    EVAL_THEFREEDICTIONARY_0: () => cmpUi.showPurposes() || cmpUi.rejectAll() || true,
    EVAL_THEFREEDICTIONARY_1: () => cmpUi.allowAll() || true,
    EVAL_USERCENTRICS_API_0: () => typeof UC_UI === "object",
    EVAL_USERCENTRICS_API_1: () => !!UC_UI.closeCMP(),
    EVAL_USERCENTRICS_API_2: () => !!UC_UI.denyAllConsents(),
    EVAL_USERCENTRICS_API_3: () => !!UC_UI.acceptAllConsents(),
    EVAL_USERCENTRICS_API_4: () => !!UC_UI.closeCMP(),
    EVAL_USERCENTRICS_API_5: () => UC_UI.areAllConsentsAccepted() === true,
    EVAL_USERCENTRICS_API_6: () => UC_UI.areAllConsentsAccepted() === false,
    EVAL_USERCENTRICS_BUTTON_0: () => JSON.parse(localStorage.getItem("usercentrics")).consents.every((c) => c.isEssential || !c.consentStatus),
    EVAL_WAITROSE_0: () => Array.from(document.querySelectorAll("label[id$=cookies-deny-label]")).forEach((e) => e.click()) || true
  };
  function getFunctionBody(snippetFunc) {
    const snippetStr = snippetFunc.toString();
    return `(${snippetStr})()`;
  }

  // lib/heuristic-patterns.ts
  var DETECT_PATTERNS = [
    /accept cookies/gi,
    /accept all/gi,
    /reject all/gi,
    /only necessary cookies/gi,
    // "only necessary" is probably too broad
    /(?:by continuing.{0,100}cookie)|(?:cookie.{0,100}by continuing)/gi,
    /(?:by continuing.{0,100}privacy)|(?:privacy.{0,100}by continuing)/gi,
    /we (?:use|serve)(?: optional)? cookies/gi,
    /we are using cookies/gi,
    /use of cookies/gi,
    /website uses cookies to enhance your browsing experience/gi,
    /(?:this|our) (?:web)?site.{0,100}cookies/gi,
    /cookies (?:and|or) .{0,100} technologies/gi,
    /such as cookies/gi,
    /read more about.{0,100}cookies/gi,
    /consent to.{0,100}cookies/gi,
    /we and our partners.{0,100}cookies/gi,
    /we.{0,100}store.{0,100}information.{0,100}such as.{0,100}cookies/gi,
    /store and\/or access information.{0,100}on a device/gi,
    /personalised ads and content, ad and content measurement/gi,
    // it might be tempting to add the patterns below, but they cause too many false positives. Don't do it :)
    // /cookies? settings/i,
    // /cookies? preferences/i,
    // FR
    /utilisons.{0,100}des.{0,100}cookies/gi,
    /des.{0,100}cookies.{0,100}pour/gi,
    /retirer.{0,100}votre.{0,100}consentement/gi,
    /et.{0,100}nos.{0,100}partenaires/gi,
    /publicités.{0,100}et.{0,100}du.{0,100}contenu/gi,
    /utilise.{0,100}des.{0,100}cookies/gi,
    /utilisent.{0,100}des.{0,100}cookies/gi,
    /stocker.{0,100}et.{0,100}ou.{0,100}accéder/gi,
    /consentement.{0,100}à.{0,100}tout.{0,100}moment/gi,
    /votre.{0,100}consentement/gi,
    /accepter.{0,100}tout/gi,
    /utilisation.{0,100}des.{0,100}cookies/gi,
    /cookies.{0,100}ou.{0,100}technologies/gi,
    /acceptez.{0,100}l.{0,100}utilisation/gi,
    /continuer sans accepter/gi,
    /tout refuser/gi,
    /(?:refuser|rejeter) tous les cookies/gi,
    /je refuse/gi,
    /refuser et continuer/gi,
    /refuser les cookies/gi,
    /seulement nécessaires/gi,
    /je désactive les finalités non essentielles/gi,
    /cookies essentiels uniquement/gi,
    /nécessaires uniquement/gi,
    // DE
    /wir.{0,100}verwenden.{0,100}cookies/gi,
    /wir.{0,100}und.{0,100}unsere.{0,100}partner/gi,
    /zugriff.{0,100}auf.{0,100}informationen.{0,100}auf/gi,
    /inhalte.{0,100}messung.{0,100}von.{0,100}werbeleistung.{0,100}und/gi,
    /cookies.{0,100}und.{0,100}andere/gi,
    /verwendung.{0,100}von.{0,100}cookies/gi,
    /wir.{0,100}nutzen.{0,100}cookies/gi,
    /verwendet.{0,100}cookies/gi,
    /sie.{0,100}können.{0,100}ihre.{0,100}auswahl/gi,
    /und.{0,100}ähnliche.{0,100}technologien/gi,
    /cookies.{0,100}wir.{0,100}verwenden/gi,
    /alles?.{0,100}ablehnen/gi,
    /(?:nur|nicht).{0,100}(?:zusätzliche|essenzielle|funktionale|notwendige|erforderliche).{0,100}(?:cookies|akzeptieren|erlauben|ablehnen)/gi,
    /weiter.{0,100}(?:ohne|mit).{0,100}(?:einwilligung|zustimmung|cookies)/gi,
    /(?:cookies|einwilligung).{0,100}ablehnen/gi,
    /nur funktionale cookies akzeptieren/gi,
    /optionale ablehnen/gi,
    /zustimmung verweigern/gi,
    // NL
    /gebruik.{0,100}van.{0,100}cookies/gi,
    /(?:we|wij).{0,100}gebruiken.{0,100}cookies.{0,100}om/gi,
    /cookies.{0,100}en.{0,100}vergelijkbare/gi,
    /(?:alles|cookies).{0,100}(?:afwijzen|weigeren|verwerpen)/gi,
    /alleen.{0,100}noodzakelijke?\b/gi,
    /cookies weigeren/gi,
    /weiger.{0,100}(?:cookies|alles)/gi,
    /doorgaan zonder (?:te accepteren|akkoord te gaan)/gi,
    /alleen.{0,100}(?:optionele|functionele|functioneel|noodzakelijke|essentiële).{0,100}cookies/gi,
    /wijs alles af/gi,
    // Spanish (ES)
    /(si|al) contin[úu]a[sr]?( navegando)?.{0,100} cookie/gi,
    /(usamos|utilizar?|utilizamos)( (tanto|las))?.{0,20}cookie/gi,
    /\b(hacemos|hace) uso de cookies\b/gi,
    /\busa cookies de google\b/gi,
    /acepta.{0,80} uso de cookies/gi,
    /al utilizar nuestro sitio web.{0,80}cookie/gi,
    /almacenar la información en un dispositivo y\/?o acceder a ella/gi,
    /cookie.{0,30} utiliza/gi,
    /cookies propias y de/gi,
    /cookies.{0,80}son necesarias/gi,
    /est[ea] (sitio|página|web)( web)?( también)? (usa|utiliza|requiere del uso de|se sirven|emplea) cookies?/gi,
    /navegando.{0,100}cookie/gi,
    /nosotros y nuestros( \d+)? (socios|proveedores).{0,180} cookies/gi,
    /recopilamos y almacenamos datos de usted y de su dispositivo/gi,
    /utilizamos tecnolog[ií]as como las cookies/gi,
    // Polish (PL)
    // examples:
    //  wykorzystuje pliki cookie (uses cookies)
    //  Wykorzystujemy informacje w plikach cookie (We use information in cookies)
    /(używamy|stosujemy|stosuje|wykorzystujemy|wykorzyst(uje|ywane))( są)?.{0,20} plik(i|ów|ach) cookie/gi,
    /(używać|używamy).{0,80} (ciasteczek|cookie)/gi,
    /cele przetwarzania twoich danych przez zaufanych partnerów iab/gi,
    /dzięki (plikom cookie|ciasteczkom|cookie)/gi,
    /korzysta.{0,80} plików cookie/gi,
    /korzystamy z technologii, takich jak pliki cookie/gi,
    /korzystamy.{0,50} cookies/gi,
    /niektóre pliki cookies/gi,
    /pliki cookies i pokrewne im technologie umożliwiają poprawne działanie strony i pomagają nam dostosować ofertę do twoich potrzeb/gi,
    /przechowywanie informacji na urządzeniu lub dostęp do nich/gi,
    /przechowywanie plików cookie na swoim urządzeniu/gi,
    /przechowywać i uzyskiwać dostęp do informacji na twoich urządzeniach/gi,
    /przetwarzamy.{0,80} cookie/gi,
    /strona.{0,50} używa (ciasteczek|cookie)/gi,
    /ta strona korzysta z ciasteczek/gi,
    /uzyskujemy dostęp i przechowujemy informacje na urządzeniu/gi,
    /używa plik[ió]w? cookie/gi,
    /używamy plików.{0,20}cookie/gi,
    /wykorzystują .{0,100}cookie/gi,
    /za pomocą plików cookies.{0,100} my lub nasi partnerzy/gi,
    /zgodą my i nasi partnerzy możemy wykorzystywać precyzyjne dane geolokalizacyjne i identyfikację/gi,
    // Catalan (CA)
    /cookies pròpies i de tercers/gi,
    /utilitzem galetes/gi,
    /\búnicament utilitza galetes pròpies amb finalitat tècnica\b/gi,
    /este lloc web utilitza només cookies tècniques necessàries per al seu funcionament/gi,
    /utilitza cookies tècniques,\s*de personalització i anàlisi/gi,
    /utilitzem cookies i altres tecnologies/gi,
    // Basque (EU)
    /cookie propio eta hirugarrenenak helburu teknikoarekin erabiltzen ditu/gi,
    /cookie propioak eta hirugarrenen cookieak erabiltzen ditugu/gi,
    /cookie propioak eta hirugarrenenak helburu teknikoarekin erabiltzen ditu/gi,
    /cookie[-\s]*ak erabiltzen ditu/gi,
    /cookieak erabiltzen ditu/gi,
    /guk eta gure \d+ bazkideek cookieak eta identifikadoreak erabiltzen ditugu/gi,
    /norberaren eta hirugarrenen cookie-?ak baino ez ditu erabiltzen/gi,
    /web orri honek cookieak erabiltzen ditu/gi,
    /webgune honek cookie propioak eta hirugarrenen cookie-fitxategiak erabiltzen ditu/gi,
    // Galician (GL)
    /^\s*empregamos cookies propias\b/gi,
    /este portal emprega cookies propias ou de terceiros con fins analíticos/gi,
    // Russian (RU)
    // e.g. "мы используем файлы cookie", "сайт использует куки", "используются технологии cookie"
    // note: \w does not match Cyrillic, so these use explicit character ranges
    /использу[а-яё]*.{0,40}(?:файл[а-яё]*[\s-]+)?(?:cookie|куки)/gi,
    /(?:cookie|куки).{0,40}использу[а-яё]*/gi,
    /(?:использовани[а-яё]*|обработк[а-яё]*|хранени[а-яё]*).{0,20}(?:файл(?:ов|ы)[\s-]+)?(?:cookie|куки)/gi,
    /технологи[а-яё]*\s+(?:cookie|куки)/gi,
    // e.g. "сайт собирает cookie", "мы применяем файлы cookie", "сохраняем куки на вашем устройстве"
    /(?:собира[а-яё]*|сбор|применя[а-яё]*|сохраня[а-яё]*|размеща[а-яё]*|устанавлива[а-яё]*).{0,30}(?:файл[а-яё]*[\s-]+)?(?:cookie|куки)/gi,
    // "файлы cookie" / "cookie-файлы" / "куки-файлы" in any phrasing
    /(?:файл[а-яё]*[\s-]+(?:cookie|куки)|(?:cookie|куки)[\s-]+файл[а-яё]*)/gi,
    // Italian (IT)
    /usiamo.{0,20}cookie/gi
  ];
  var REJECT_PATTERNS_ENGLISH = [
    // e.g. "reject", "reject all", "reject all cookies", "deny all", "refuse cookies", "decline",
    // "reject non-essential cookies", "reject unnecessary cookies", "reject all but necessary", "reject all and close"
    // note that "reject and subscribe" and "reject and pay" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    /^\s*(no,?\s*)?(i\s+)?(reject|deny|refuse|decline|disable)\s*(all)?\s*(but|except)?\s*(non[- ]?essential|un(necessary|required)|optional|additional|targeting|analytics|marketing|non[- ]?necessary|extra|tracking|advertising|necessary|essential)?\s*(cookies)?\s*(and\s+close)?\s*$/is,
    // e.g. "i do not accept", "do not accept cookies", "i do not accept the use of cookies"
    /^\s*(no,?\s*)?(i\s+)?do\s+not\s+accept\s*(the\s+use\s+of\s+)?(any\s+)?(cookies)?\s*$/is,
    // e.g. "continue without accepting", "continue without agreeing", "continue without agreeing →"
    /^\s*(continue|proceed|continue\s+browsing)\s+without\s+(accepting|agreeing|consent|cookies|tracking)(\s*→)?\s*$/is,
    // essential/necessary/functional-only, e.g. "essential cookies only", "accept only essential cookies",
    // "allow necessary cookies continue", "use essential cookies only", "functional only", "i confirm necessary"
    /^\s*(i\s+)?(want\s+to\s+)?(only\s+)?(use|accept|allow|keep|enable|choose|continue\s+with|i\s+confirm)\s*(only\s+)?(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s*(cookies)?\s*(continue|only)?\s*$/is,
    /^\s*(i\s+)?(want\s+to\s+)?only\s+(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s*(cookies)?\s*(continue|only)?\s*$/is,
    /^\s*(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s*(cookies)?\s+only\s*$/is,
    // e.g. "do not sell or share my personal information", "opt out of sale ..." (CCPA)
    /do\s+not\s+sell|opt\s+out\s+of\s+sale/is,
    // e.g. "opt-out of sale/share or targeted advertising", "opt-out of advertising/social media cookies"
    /^opt[ -]?out of /is,
    // e.g. "reject all except strictly necessary", "reject all (except necessary cookies)"
    /except\s+(strictly\s+)?necessary/is,
    // e.g. "disagree", "i disagree", "disagree and close"
    /^(i\s+)?disagree\s*(and\s+close)?$/i,
    "no",
    /^no,? thank(s| you)$/is,
    /^opt[ -]out$/is,
    "dont enable",
    "withdraw consent",
    "i do not agree"
  ];
  var REJECT_PATTERNS_DUTCH = [
    // weigeren / afwijzen / verwerpen (reject verbs, any position)
    /weiger|afwijz|verwerp/is,
    // "wijs (alles/ze) af" (reject verb with split particle)
    /wijs\b.*\baf\b/is,
    // "alleen/enkel/uitsluitend ... (noodzakelijk|functioneel|essentieel|...)"
    /(^|\s)(alleen|enkel|uitsluitend)\s+.{0,20}(noodzakelijk|functione|essenti|vereiste|verplichte|strikt|minimale|basis|basic|standaard)/is,
    // essential/necessary-only nouns
    /^\s*(accepteer\s+|aanvaard\s+|gebruik\s+|sta\s+|ik wil\s+)?(alleen\s+|enkel\s+|uitsluitend\s+|strikt\s+)?(de\s+)?(noodzakelijke?|functionele?|functioneel|essentiële|essentieel|vereiste|verplichte|minimale|basiscookies|basis|standaard)\s*(cookies?)?\s*(accepteren|toestaan|aanvaarden)?\s*$/is,
    // continue without accepting / consent
    /(doorgaan|ga door|ga verder|verder)\s+.{0,15}(zonder|aanvaard)/is,
    // "nee" refusals (but not "nee, sluiten" → acknowledge)
    /^nee(,?\s+(bedankt|dank je|dankje|liever niet|liever geen cookies|geen persoonlijke cookies|geen cookies.*|weigeren?))?$/is,
    "niet accepteren",
    "niet akkoord",
    "ik ga niet akkoord",
    "liever niet",
    "geen cookies toestaan",
    "liever geen cookies",
    "functioneel altijd actief"
  ];
  var REJECT_PATTERNS_FRENCH = [
    // refuser / rejeter / interdire / décliner (reject verbs, any position)
    // "refuser et s'abonner" / "refuser et payer" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    /(^|\s)(refus|rejet|rejeter|interdire|interdis|déclin|declin)/is,
    // only necessary / essential / technical / functional
    /(uniquement|seulement|indispensable|strictement nécessaire|que les cookies\s+(nécessaires|techniques|essentiels|indispensables|fonctionnels))/is,
    // continue/proceed without accepting; refuse everything; disable purposes
    /(sans accepter|ne pas accepter|je naccepte rien|je désactive)/is,
    // "non" / "non, merci"
    /^non(,?\s+merci\.?)?$/is
  ];
  var REJECT_PATTERNS_GERMAN = [
    // "... ablehnen" / "ablehnen ..." (reject/decline). Exclude the settings-list phrase "einstell(ungen|en) oder ablehnen".
    /^(?!einstell(ungen|en) oder ablehnen$).*ablehnen/is,
    // verweigern / verweigere / verweigert (refuse)
    /verweiger/is,
    // essential/necessary/functional-only variants (accepting only necessary → reject)
    /^\s*(nur|ausschließlich|lediglich|weiter\s+mit|mit|akzeptiere?n?|unbedingt|es\s+werden\s+nur)?\s*(technisch\s+)?(notwendige?[nrs]?|essenzielle?[nrs]?|essentielle?[nrs]?|erforderliche?[nrs]?|funktionale?[nrs]?|funktionelle?[nrs]?|wesentliche?[nrs]?)\s*(cookies?|technologien|funktionscookies|dienste)?\s*(akzeptieren|erlauben|zulassen|verwenden|annehmen|setzen|speichern|zustimmen|auswählen)?\.?\s*$/is,
    // continue without consent
    /(^|\s)(ohne\s+(zu\s+)?(einwilligung|zustimmung|einverständnis|annahme|annehmen|akzeptanz|akzeptieren)|(weiter|fortfahren)\s+ohne)/is,
    // negations / refusals not covered by the regexes above
    "nein, danke",
    "nein, bitte nicht",
    "nein, ich stimme nicht zu",
    "nicht zustimmen",
    "nicht einverstanden",
    "ich lehne ab",
    "mit erforderlichen einstellungen fortfahren",
    "mit erforderlichen cookies fortfahren",
    "mit notwendigen fortfahren"
  ];
  var REJECT_PATTERNS_ITALIAN = [
    // rifiuta / rifiutare / rifiuto / nega / negare / blocca (reject verbs)
    /(^|\s)(rifiut|neg(a|are|hi)|blocca i cookie)/is,
    // accept/use only necessary / essential / technical
    /^\s*(accetta|accettare|usa|installa|consenti|chiudi e prosegui( solo con)?)?\s*(solo|soltanto|unicamente)?\s*(i\s+|gli\s+)?(cookies?\s+)?(strettamente\s+)?(necessari|necessary|essenziali|tecnici|di navigazione)\s*(necessari|tecnici|essenziali)?\s*$/is,
    // continue without accepting
    /continua(re)? senza accettare/is,
    "non accetto"
  ];
  var REJECT_PATTERNS_BRAZILIAN_PORTUGUESE = [
    // (deny)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*$/is,
    // (proceed) (without accepting)
    /^\s*(continuar|prosseguir|seguir)\s*(sem\s*aceitar)\s*$/is,
    // (deny) (everything) (optional)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*(tudo|o)?\s*(opcional|(não[-\s](essencial|funcional|obrigatório|necessário)))?\s*$/is,
    // (deny) (all) (the) (optional) (cookies)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*(todos)?\s*(os)?\s*(cookies)?\s*(opcionais|(não[-\s](essenciais|funcionais|obrigatórios|necessários)))?\s*$/is,
    // (accept) (only) (the) (essential)
    /^\s*(aceitar|utilizar)?\s*(apenas|somente|só)?\s*(o)?\s*(essencial|funcional|obrigatório|necessário)\s*$/is,
    // (accept) (only) (the) (essential) (cookies)
    /^\s*(aceitar|utilizar)?\s*(apenas|somente|só)?\s*(os)?\s*(cookies)?\s*(essenciais|funcionais|obrigatórios|necessários)\s*$/is
  ];
  var REJECT_PATTERNS_SPANISH = [
    // rechazar / denegar / declinar / negar (reject verbs, any position)
    // "rechazar y pagar" / "rechazar y suscribirse" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    /(^|\s)(rechaz|recház|deneg|negar|declin)/is,
    // accept/allow/use (only) necessary / essential / technical / functional / own
    /^\s*(aceptar?|acepta|permitir|permite|usar|utilizar)?\s*(solo|sólo|només|únicamente)?\s*(las?\s+|los\s+)?(cookies?\s+)?(estrictamente\s+)?(necesari\w*|esencial\w*|técnic\w*|obligatori\w*|funcional\w*|propias)\s*$/is,
    // "solo/sólo/no, sólo ... necessary/essential"
    /^(no,?\s+)?(solo|sólo|només)\s+(usar\s+|las?\s+|los\s+|lo\s+)?.{0,20}(necesari|esencial|estrictamente)/is,
    // refusals / opt-outs
    /^(no acept|no consentir|no permitir|no estoy de acuerdo|no,? gracias|sin consentimiento|revocar consentimiento|continuar sin aceptar|prefiero rechazarlas|descartar todas)/is,
    "acceptar nom\xE9s les necess\xE0ries",
    "nom\xE9s sutilitzen cookies quan \xE9s necessari",
    "pulsa aqu\xED para desactivar las cookies opcionales"
  ];
  var REJECT_PATTERNS_SWEDISH = [
    // avvisa / avböj / neka / förneka (reject verbs)
    /(^|\s)(avvisa|avböj|neka|nekar|förneka)/is,
    // (allow/accept/use) only necessary cookies/kakor
    /(bara|endast|enbart)\s+nödvändig/is,
    // "godkänn/acceptera/använd/tillåt (bara/endast/enbart) nödvändiga (cookies/kakor)"
    /^(ok,?\s+|nej,?\s+)?(jag\s+)?(godkänn\w*|godta|acceptera\w*|använd\w*|tillåt|spara)?\s*(bara|endast|enbart)?\s*(strikt\s+)?nödvändiga?t?( (cookies|kakor|tjänster))?\.?$/is,
    // continue without accepting
    /fortsätt utan att (acceptera|godkänna)/is,
    /strikt nödvändig/is,
    "till\xE5t inte cookies",
    "jag accepterar endast grundl\xE4ggande kakor"
  ];
  var REJECT_PATTERNS_CATALAN = [/(^|\s)rebutj/is, "no accepto", "no, gr\xE0cies"];
  var REJECT_PATTERNS_GALICIAN = [/(^|\s)rexeitar/is];
  var REJECT_PATTERNS_BASQUE = [/(^|\s)(baztertu|ukatu)/is];
  var REJECT_PATTERNS_PORTUGUESE = [/^aceitar apenas cookies essenciais\.$/];
  var REJECT_PATTERNS_CZECH = ["povolit pouze nezbytn\xE9 cookie"];
  var REJECT_PATTERNS_POLISH = [
    // odrzuć / odrzucam / odmawiam / rezygnuję / blokuj wszystkie (reject verbs)
    /odrzu(ć|cam|cenie|cać|canie|cić)|odmaw|odmowa|odmów|rezygnuj|blokuj wszystk/is,
    // (accept) only necessary / required
    /(^|\s)tylko\s+(bezwzględnie\s+)?(niezbędn\w*|wymagan\w*|konieczne)/is,
    /(akceptuj|akceptuję|zaakceptuj|zatwierdź|potwierdzam|zezwól)\s+(tylko\s+)?(na\s+)?(niezbędn\w*|wymagan\w*|konieczne)/is,
    /korzystaj wyłącznie z niezbędn/is,
    // continue without accepting / consent
    /kontynuuj bez (akceptacj|akceptowani|wyrażania zgody)/is,
    // refusals
    /nie (akceptuję|zgadzam|wyrażam zgody|wyrażaj zgody|zezwalaj|potwierdzam)/is,
    /^nie(,?\s+(dziękuję|nie zgadzam.*))?$/is,
    "niezb\u0119dne",
    "niezb\u0119dne pliki cookie",
    /^funkcjonalne pliki cookie \(wymagane\)$/
  ];
  var REJECT_PATTERNS_RUSSIAN = [
    // отклонить / отказаться / запретить (reject verbs); \w does not match Cyrillic
    /(^|\s)(отклон[а-яё]*|отказ[а-яё]*|откаж[а-яё]*|запрет[а-яё]*|запрещ[а-яё]*)/is,
    // "только необходимые (файлы cookie)" / "принять только необходимые куки"
    /^\s*(принять\s+|принима[а-яё]+\s+|разрешить\s+|использовать\s+|оставить\s+)?(только|лишь)\s+(строго\s+)?(необходим[а-яё]+|нужн[а-яё]+|обязательн[а-яё]+|техническ[а-яё]+|функциональн[а-яё]+)(\s+файл[а-яё]*)?([\s-]+(cookie|куки)([\s-]+файл[а-яё]*)?)?\s*$/is,
    // refusals: "не принимаю", "не согласен", "не соглашаюсь", "нет, спасибо"
    /^\s*(не\s+(принима[а-яё]+|соглас[а-яё]+|разреша[а-яё]+|хочу)|нет(,?\s+спасибо)?)\s*$/is
  ];
  var REJECT_PATTERNS_TURKISH = ["reddet", "\xE7erezleri reddet"];
  var REJECT_PATTERNS_INDONESIAN = ["tolak cookie"];
  var REJECT_PATTERNS = [
    ...REJECT_PATTERNS_ENGLISH,
    ...REJECT_PATTERNS_DUTCH,
    ...REJECT_PATTERNS_FRENCH,
    ...REJECT_PATTERNS_GERMAN,
    ...REJECT_PATTERNS_ITALIAN,
    ...REJECT_PATTERNS_BRAZILIAN_PORTUGUESE,
    ...REJECT_PATTERNS_SPANISH,
    ...REJECT_PATTERNS_SWEDISH,
    ...REJECT_PATTERNS_CATALAN,
    ...REJECT_PATTERNS_GALICIAN,
    ...REJECT_PATTERNS_BASQUE,
    ...REJECT_PATTERNS_PORTUGUESE,
    ...REJECT_PATTERNS_CZECH,
    ...REJECT_PATTERNS_POLISH,
    ...REJECT_PATTERNS_RUSSIAN,
    ...REJECT_PATTERNS_TURKISH,
    ...REJECT_PATTERNS_INDONESIAN
  ];
  var BUTTON_NEVER_MATCH_PATTERNS = [
    /pay|subscribe/is,
    /abonneer/is,
    /abonnier/is,
    /abonner/is,
    /abbonati/is,
    /iscriviti/is,
    /abbonare/is,
    /iscrivere/is,
    /sostienici/is,
    /suscribir/is,
    // Spanish (ES)
    /^abandonar este sitio$/,
    /suscribo/,
    /^accede gratis con cookies publicitarias$/,
    /pagar/,
    /suscríbete/,
    /sin cookies .{0,10}euro/s,
    // Polish (PL)
    /subskrybuj/,
    // Russian (RU): paywall/subscription wording, e.g. "отказаться от подписки", "оплатить"
    /подписатьс|подписк|оплатит|оплачива/is
  ];
  var DETECT_NEVER_MATCH_PATTERNS = [
    // e.g. "age verification", "age confirmation", "age check", "age gate", "age restriction"
    /age\s+(?:verification|confirmation|check|gate|restriction)/i,
    // e.g. "over 18 years", "at least 21 years", "older than 21", "18+"
    /(?:over|above|at\s*least|minimum|older\s+than)\s*(?:18|21)\s*(?:years|yo|y\.?o\.?|\+)?/i,
    // e.g. "18 years of age", "18+ years old", "21 years or older"
    /(?:18|21)\s*(?:\+|years?)\s*(?:of\s*age|or\s*older|or\s*above)/i,
    // e.g. "I am 18+", "I am over 18", "I'm 21 or older"
    /(?:i'?m|i\s*am)\s*(?:over|above|at\s*least)?\s*(?:18|21)(?:\+|\s*(?:or\s*older|years))?/i,
    // e.g. "you must be 18", "users must be at least 21", "visitors must be over 18"
    /(?:you|users?|visitors?)\s+must\s+be\s+(?:over|above|at\s*least)?\s*(?:18|21)/i,
    // e.g. "adult oriented material", "adult content", "adult-only website"
    /adult[\s-]+(?:oriented|only|content|material|websites?)/i
  ];
  var SETTINGS_PATTERNS = [
    // Multilingual "open customization" patterns: a customization verb next to a
    // cookie/preference/settings/options/details/purposes noun (both word orders).
    // The negative lookahead avoids policy links and save/confirm/accept phrases (those are accept/acknowledge/other).
    /^(?!.*\b(policy|policies|notice|statement|impressum|richtlinie|beleid|politique|política|polityk|speichern|guardar|opslaan|zapisz|enregistrer|sauvegarder|bevestig|bestätig|confirm|save|submit|akzeptier|accept|zaakcept)\b)(customi[sz]e|manage|adjust|configure|personali[sz]e|let me choose|edit|change|set|select|view|see|review|update|open|show|choose|anpassen|verwalten|konfigurieren|bearbeiten|öffnen|anzeigen|einblenden|festlegen|auswählen|wählen|aanpassen|beheren|instellen|wijzig\w*|personaliseren|personaliseer|kies|bekijk|toon|personnaliser|paramétrer|gérer|configurer|choisir|afficher|définir|modifier|configurar|personalizar|gestionar|administrar|ajustar|seleccionar|modificar|establecer|dostosuj|zarządzaj|personalizuj|ustaw\w*|zmień|pokaż|wybierz)\b.{0,20}(cookies?|preferences?|settings?|options?|choices?|controls?|details?|purposes?|services?|consent|einstellung\w*|optionen|präferenzen|einzelheiten|zwecke|dienste|datenschutz\w*|auswahl|voorkeur\w*|instelling\w*|opties|diensten|préférences|paramètres|réglages|choix|détails|finalités|témoins|preferencia\w*|opciones|ajustes|configuraci\w*|detalles|servicios|elección|preferencj\w*|ustawie\w*|opcje|szczegół\w*|cele|galetes)\b/is,
    /^(?!.*\b(policy|policies|notice|statement|impressum|richtlinie|beleid|politique|política|polityk|speichern|guardar|opslaan|zapisz|enregistrer|sauvegarder|bevestig|bestätig|confirm|save|submit|akzeptier|accept|zaakcept)\b)(cookies?|preferences?|settings?|options?|choices?|controls?|details?|purposes?|services?|consent|einstellung\w*|optionen|präferenzen|einzelheiten|zwecke|dienste|datenschutz\w*|auswahl|voorkeur\w*|instelling\w*|opties|diensten|préférences|paramètres|réglages|choix|détails|finalités|témoins|preferencia\w*|opciones|ajustes|configuraci\w*|detalles|servicios|elección|preferencj\w*|ustawie\w*|opcje|szczegół\w*|cele|galetes)\b.{0,15}(customi[sz]e|manage|adjust|configure|personali[sz]e|let me choose|edit|change|set|select|view|see|review|update|open|show|choose|anpassen|verwalten|konfigurieren|bearbeiten|öffnen|anzeigen|einblenden|festlegen|auswählen|wählen|aanpassen|beheren|instellen|wijzig\w*|personaliseren|personaliseer|kies|bekijk|toon|personnaliser|paramétrer|gérer|configurer|choisir|afficher|définir|modifier|configurar|personalizar|gestionar|administrar|ajustar|seleccionar|modificar|establecer|dostosuj|zarządzaj|personalizuj|ustaw\w*|zmień|pokaż|wybierz)\b/is,
    "settings",
    "preferences",
    /customi(s|z)e/is,
    "more options",
    /(manage|configure) (my|your) (preferences|choices|cookies)/is,
    /(cookie )?preference center/is,
    "configure",
    "cookie manager",
    "cookie preference",
    "let me choose",
    "cookieconsent preferences",
    /privacy choices/is,
    /^(privacy|cookie|custom) settings$/is,
    /^cookies? (settings|preferences|setting)$/is,
    /(manage|customize|customise|opt-out|edit).*(cookies|preferences|settings|options)/is,
    "cookie consent options",
    "privacy controls",
    // German
    "einstellungen",
    // Spanish (ES)
    /^(configurar|configuración|administrar)$/,
    /^(gestionar|ver|establecer) preferencias$/,
    "ajustes",
    "centro de preferencias",
    "configura",
    /^configuración( de( las)?)? cookies?( y servicios)?$/,
    /^configurar\.\.\.$/,
    "configurarlas",
    "detalles",
    "gestiona tus preferencias",
    /^gestionar ?(las?|mis)? ?(configuración|preferencias)?(( de)? cookies?)?$/,
    "gesti\xF3n cookies",
    "gesti\xF3n de cookies",
    "mis preferencias",
    "mostrar detalles",
    "mostrar los prop\xF3sitos",
    "m\xE1s opciones",
    "no, ajustar",
    "obtener m\xE1s informaci\xF3n y configuraci\xF3n",
    "opciones de gesti\xF3n",
    "panel de configuraci\xF3n de cookies",
    "personalice",
    "personalizar",
    "preferencias de privacidad",
    "preferencias",
    "quiero configurarlas",
    "saber m\xE1s y personalizar",
    "seleccionar fines individuales",
    // Catalan (CA)
    "configura-les",
    "personalitza",
    "veure prefer\xE8ncies",
    // Galician (GL)
    "xestionar preferencias",
    // Basque (EU)
    /^(konfigurazioa|konfiguratu)$/,
    // Portuguese (PT)
    "gerenciar cookies",
    // French (FR)
    "param\xE9trage des cookies",
    "param\xE9trer",
    "personnaliser",
    "param\xE8tres",
    "pr\xE9f\xE9rences",
    "r\xE9glages",
    "d\xE9tails",
    "gestion des cookies",
    /^gérer (les |mes )?cookies$/,
    "je choisis",
    "voir les pr\xE9f\xE9rences",
    // German (DE)
    "abschnitt einzelheiten",
    "cookie-details",
    /^datenschutz-?einstellungen$/,
    "cookie-einstellungen",
    /^einstell(ungen|en) oder ablehnen$/,
    "ausw\xE4hlen",
    /^einstellungen (anpassen|ansehen|verwalten|ändern)$/,
    "erweiterte einstellungen",
    "individuelle datenschutz-pr\xE4ferenzen",
    "individuelle datenschutzeinstellungen",
    "konfigurieren",
    "mehr optionen",
    "pr\xE4ferenzen",
    "individuelle einstellungen",
    "privatsph\xE4re einstellungen",
    // Dutch (NL)
    /^(aan|an)passen$/,
    /^cookie[- ]instellingen$/,
    "cookiestatement instellingen",
    /^details (tonen|weergeven)$/,
    "instellingen",
    "meer opties",
    "zelf instellen",
    // Czech (CS)
    "podrobn\xE9 nastaven\xED",
    // Polish (PL)
    // examples:
    //  dostosuj pliki cookie (adjust cookies)
    //  zarządzaj plikami cookie (manage cookies)
    /^(dostosuj|s?personalizuj|chcę dostosować|zarządzaj) ?(moje|moimi)? ?(ustawieniami|preferencjami)? ?(zgody|wybory|(plik(i|ami|ów))? cookies?)?$/,
    /^(preferencje|zarządzaj preferencjami)$/,
    /^(ustawienia|zmień ustawienia|zmiana ustawień|zarządzaj opcjami)$/,
    "centrum preferencji",
    /^chcę dokonać ustawień cookies\.$/,
    "dostosuj wyb\xF3r",
    "edytuj ustawienia",
    "konfiguracja zg\xF3d",
    "otw\xF3rz ustawienia",
    "personalizacja",
    "poka\u017C cele",
    "poka\u017C szczeg\xF3\u0142y",
    "szczeg\xF3\u0142y",
    "pozw\xF3l mi wybra\u0107",
    /^przejdź do ustawień plików cookies\.$/,
    "przejd\u017A do ustawie\u0144 prywatno\u015Bci",
    "przejd\u017A do ustawie\u0144",
    "skonfiguruj",
    "ustaw swoje wybory",
    "ustawienia ciasteczek",
    "ustawienia prywatno\u015Bci",
    "ustawienia zaawansowane",
    "ustawienia zgody",
    /^ustawienia(ch)?( plików)? cookies?$/,
    "ustawieniach",
    "ustawie\u0144 zaawansowanych",
    "wi\u0119cej opcji",
    "wi\u0119cej ustawie\u0144",
    /^wybierz, jakie pliki cookies chcesz zaakceptować\.$/,
    "zaawansowane",
    "zarz\u0105dzaj zgodami dotycz\u0105cymi plik\xF3w cookies",
    "zarz\u0105dzaj zgodami",
    "zarz\u0105dzania zgodami",
    "zarz\u0105dzanie opcjami",
    "zarz\u0105dzanie preferencjami",
    "zarz\u0105dzanie ustawieniami plik\xF3w cookie",
    "zmieniam ustawienia",
    "zmieniam zgody",
    "zmie\u0144 swoje preferencje",
    /^zmień ustawienia( plików)? cookies?$/,
    "zmie\u0144 zgody",
    "zobacz preferencje",
    // Russian (RU)
    // e.g. "настроить", "настройки куки", "параметры конфиденциальности"; \w does not match Cyrillic
    /^\s*((мои|моими|свои|своими)\s+)?(настро[а-яё]+|парамет[а-яё]+|предпочтени[а-яё]+)([\s-]+(мои|моими|свои|своими|файл[а-яё]*|cookie|куки[а-яё]*|конфиденциальност[а-яё]*|согласи[а-яё]*|приватност[а-яё]*))*\s*$/is,
    // e.g. "управление файлами cookie", "изменить настройки", "выбрать категории"
    // "подробн" is deliberately excluded: it would also match the "подробнее о cookie" policy link
    /^\s*(управл[а-яё]+|измен[а-яё]+|выбрать|персонализ[а-яё]+|расширенн[а-яё]+|индивидуальн[а-яё]+)[\s-]+.{0,20}(настройк[а-яё]*|парамет[а-яё]*|предпочтени[а-яё]*|категори[а-яё]*|файл[а-яё]*|cookie|куки[а-яё]*|согласи[а-яё]*|конфиденциальност[а-яё]*)\s*$/is,
    "\u043F\u043E\u0434\u0440\u043E\u0431\u043D\u044B\u0435 \u043D\u0430\u0441\u0442\u0440\u043E\u0439\u043A\u0438",
    // Italian (IT)
    "personalizza cookie",
    // English (EN)
    "advanced settings",
    "consent settings",
    /^details (anzeigen|zeigen|section)$/,
    "no, adjust",
    "personalize",
    "plus doptions",
    "privacy manager"
  ];
  var ACCEPT_PATTERNS = [
    // EN accept/agree/allow/consent/enable (+ all/cookies/selection/continue/close/proceed...).
    // The negative lookahead avoids essential-only/reject wording (those are reject).
    /^(?!.*\b(essential|necessary|required|functional|minimal|reject|deny|refuse|decline|only|dismiss)\b)(?!.*all cookies continue)(yes,?\s+)?(i\s+)?(accept|agree|allow|consent|enable)(\s+(to\s+)?(all( cookies)?|cookies|selection|selected( cookies)?|everything|recommended( cookies| settings)?|optional( cookies)?|additional cookies|analytics cookies))?(\s+(and\s+)?(continue|close|proceed|save))?\s*$/is,
    /^continue (and accept|using cookies|with (all|recommended cookies|cookies))$/is,
    // DE accept verbs
    /^(alle[sn]?\s+|allem\s+|ich\s+|cookies\s+|ausgewählte\s+|webanalyse\s+)?(cookies?\s+)?(akzeptieren|annehmen|zustimmen|zulassen|erlauben|einwilligen|aktivieren|auswählen)(\s+(und\s+)?(weiter|schließen))?\s*$/is,
    /^((meine\s+)?auswahl|alle)\s+(bestätigen|akzeptieren|auswählen)$/is,
    /^(alle[nm]?\s+)?(zustimmen|einverstanden|einwilligung|zustimmung)$/is,
    /^ich (bin einverstanden|akzeptiere( alle)?|stimme zu)$/is,
    // NL accept verbs
    /^(ja,?\s+)?(alle[s]?\s+|ik\s+)?(cookies?\s+)?(accepteer|accepteren|toestaan|aanvaard|aanvaarden|ga akkoord|akkoord)(\s+(en\s+(sluiten|doorgaan|verdergaan)|cookies|alle))?\s*$/is,
    /^(selectie (accepteren|toestaan)|accepteer (selectie|alle)|alle (toestaan|accepteren|aanvaarden)|ja, (dat is prima|prima|alles toestaan|accepteren|ik accepteer cookies|ik ga akkoord)|is goed)$/is,
    // FR accept verbs
    /^(oui,?\s+)?(je\s+)?(tout\s+)?(accepter|jaccepte|autoriser)(\s+(tout|tous les (cookies|témoins)|les (cookies|témoins)|la sélection|et (continuer|fermer|poursuivre)))?\s*$/is,
    /^(oui, (jaccepte|je suis daccord)|jaccepte (les cookies|lutilisation de cookies)|accepter (continuer|et poursuivre)|continuer et accepter|fermer et accepter)$/is,
    // ES/CA accept verbs
    /^(sí,?\s+|si,?\s+)?(aceptar|acepta|permitir|permitirlas|consentir|estoy de acuerdo|de acuerdo|estic dacord)(\s+(todo|todas( las cookies)?|cookies|la selección|selección|y (cerrar|continuar|seguir( leyendo)?|leer gratis)))?\s*$/is,
    // save / submit selection / preferences (accept semantics; acknowledge catches "guardar configuración/selección" first)
    /^(save|store|submit|guardar|sauvegarder|zapisz|opslaan|bewaar)\b.{0,25}(preference|setting|selection|choice|mes choix|cookie|voorkeur|keuze|ustawien|zgod)/is,
    /^((meine\s+)?auswahl|einstellungen|einwilligung|voorkeuren|instellingen|selectie|keuzes)\s+(speichern|opslaan)$/is,
    /i (accept|allow)( all)?/is,
    "yes",
    /accept all above/is,
    "close and accept",
    /accept all$/is,
    "im ok with that",
    // Spanish (ES)
    /^acept(ar|o)( cookies)?$/,
    /^acept(o|ar) todas las cookies$/,
    "aceptar cookies opcionales",
    "aceptar gratis",
    "aceptar las cookies",
    "aceptar todas cookies",
    "aceptar todas y cerrar",
    "aceptar todas y continuar",
    "aceptar todo y cerrar",
    /^aceptar y (continuar|seguir|navegar)( gratis)?$/,
    "aceptar y seguir navegando",
    "aceptarlas todas",
    "guardar preferencias",
    "ok, las acepto",
    /^s[íi], acepto todas las cookies$/,
    /^s[íi], acepto$/,
    /^s[íi], estoy de acuerdo$/,
    "x aceptar y cerrar",
    // Catalan (CA)
    /^accept(ar|o)( cookies)?$/,
    "accepta",
    "accepta totes les cookies",
    "accepta-ho tot",
    "accepta-les totes",
    "acceptar galetes",
    "acceptar i tancar",
    "acceptar tot",
    "acepta-les totes",
    "permet-les totes",
    "permetre totes les cookies",
    "permetre la selecci\xF3",
    // Basque (EU)
    /^denak? onartu$/,
    /^onartu \(cookie\)$/,
    "onartu cookieak",
    "onartu",
    // Portuguese (PT)
    "aceitar cookies",
    "aceitar",
    "de acordo",
    // French (FR)
    /^accepter (tout|tous les cookies|fermer)$/,
    // German (DE)
    /^(alles akzeptieren|alle zulassen|auswahl erlauben|cookies zulassen|einverstanden|einwilligung|zustimmen|zustimmung)$/,
    // Dutch (NL)
    /^(accepteer (alles|alle cookies)|alles (accepteren|toestaan)|alle cookies (accepteren|toestaan))$/,
    // Czech (CS)
    "souhlas\xEDm",
    // Polish (PL)
    // examples:
    //  akceptuj cookies (Accept cookies)
    //  akceptuj wszystkie pliki cookie (Accept all cookies)
    /^(zaakceptuj|akceptuj[eę]|akceptuj) ?(wszystkie|wszystko)?( pliki)? ?(zgody|ciasteczka|cookies?)?$/,
    "akceptowanie plik\xF3w cookie",
    "akceptuj wybrane",
    "akceptuj i zamknij",
    "akceptuj wszystkie i przejd\u017A do serwisu",
    "akceptuj\u0119 i przechodz\u0119 do serwisu",
    "akceptuj\u0119 polityk\u0119 plik\xF3w cookies i przechodz\u0119 do strony",
    "akceptuj\u0119 ustawienia cookies",
    "akceptuj\u0119 wszystkie i korzystam z us\u0142ug",
    "akceptuj\u0119!",
    "ok, zgadzam si\u0119",
    "potwierdzam wszystkie",
    "przejd\u017A do serwisu",
    "tak",
    "tak, zgadzam si\u0119 na wszystkie pliki cookie",
    "tak, zgadzam si\u0119",
    "wyra\u017A zgod\u0119 na wszystko",
    "wyra\u017Cam zgod\u0119 na wszystkie",
    "wyra\u017Cam zgod\u0119",
    "w\u0142\u0105cz wszystkie ciasteczka",
    "zaakceptuj i kontynuuj",
    "zaakceptuj i zamknij",
    "zaakceptuj wszystkie i przejd\u017A do serwisu",
    "zaakceptuj wszystkie zgody i wejd\u017A do serwisu",
    "zaakceptuj wszystkie zgody i zapisz",
    "zatwierd\u017A",
    "zezwolenie na wszystkie",
    "zezw\xF3l na wszystkie ciasteczka",
    "zezw\xF3l na wszystkie cookies",
    "zezw\xF3l na wszystkie pliki cookies",
    "zezw\xF3l na wszystkie",
    "zezw\xF3l na wyb\xF3r",
    "zezw\xF3l",
    "zgadzam si\u0119 na wszystkie",
    "zgadzam si\u0119",
    "zgoda na wszystkie",
    "zgoda",
    "zaakceptuj wybrane",
    "zezw\xF3l na wybrane",
    "zgoda na wybrane",
    // Russian (RU)
    // e.g. "принять всё", "принять все файлы cookie", "разрешить куки", "я согласен"
    // confirm/save wording (подтвердить, сохранить) is acknowledge, not accept
    /^\s*(да,?\s+)?(я\s+)?(принять|принима[а-яё]+|соглас[а-яё]+|разрешить|разреша[а-яё]+)(\s+(вс[её]|(все\s+)?(файлы[\s-]+)?(cookie|куки)([\s-]+файл[а-яё]*)?|выбранные|выбор))?\s*$/is,
    "\u043F\u0440\u0438\u043D\u044F\u0442\u044C",
    // Turkish (TR)
    "kabul et",
    // Italian (IT)
    "accetta",
    "accetta tutti i cookie"
  ];
  var ACKNOWLEDGE_PATTERNS = [
    // close / dismiss the banner/dialog/message (multilingual). The negative lookahead avoids
    // accept/save phrases (e.g. "agree and close", "akkoord en sluiten", "speichern schließen").
    /^(?!.*\b(accept\w*|accepter|accepteer|accepteren|agree|allow|akkoord|aanvaard\w*|zustimm\w*|annehm\w*|akzeptier\w*|aceptar|acepta|permit\w*|consent\w*|einverstanden|zezw\w*|zgadzam|zgoda|guardar|opslaan|enregistrer|speicher\w*|zapisz)\b)(x\s+|nee,?\s+)?(close|dismiss|schlie(ß|ss)en|sluiten|afsluiten|fermer|cerrar|tanca|beenden|masquer|zamknij)( (this|the|ce|le|el|de|des|het|la|een)?\s*(banner|bandeau|banier|bar|dialog|dialogue|window|okno|melding|message|notification|informa\w*|notificaci\w*|cookie\w*|bannière|rgpd|gdpr|hier|des cookies|de cookies|x))*\.?\s*$/is,
    // "ok" / "okay" / "oké" (optionally followed by a short acknowledgement)
    /^(ok|okay|oké|okey|k)([ .!,]*)(got it|verstanden|compris|rozumiem|thanks|gracias|ik begrijp( dat| het)?|continue to website|pour moi|fermer)?[ .!]*$/is,
    // "understood" / "got it" / "that's ok" (multilingual)
    /^(i understand|understood|got it|thats (ok|fine|okay)|alright|alles klar|in ordnung|verstanden|begrepen|jai compris|je comprends|compris|ik begrijp het|ik snap het|entendido|c(e)?st ok pour moi)[ !.,]*(merci|bedankt|dismiss this banner)?[ !.]*$/is,
    // confirm
    /^(confirm|bestätigen|bevestigen|confirmar|potwierdź)[ !.]*$/is,
    // neutral "continue" without accept/reject wording
    /^(continuer|doorgaan|ga verder)$/is,
    "continue",
    "x",
    /^got it!?$/,
    "acknowledge",
    /^close (banner|cookie notification)$/is,
    /understood$/is,
    "confirm my choices",
    // French (FR)
    "accepter fermer",
    // German (DE)
    "akzeptieren schlie\xDFen",
    "speichern schlie\xDFen",
    // Spanish (ES)
    /^.?( lo)?(entendido|entiendo).?$/s,
    "aceptar seleccionadas",
    "continuar",
    "guardar configuraci\xF3n",
    "guardar selecci\xF3n",
    "guardar y cerrar",
    "ir al contenido principal",
    "seguir",
    "vale",
    "\xA1vamos!",
    // Catalan (CA)
    "dacord",
    // Polish (PL)
    "kontynuuj",
    "ok, zrozumia\u0142em",
    /^ok.? rozumiem.?$/s,
    "rozumiem!",
    "rozumiem",
    "rozumiem, nie pokazuj wi\u0119cej",
    "w porz\u0105dku!",
    "w porz\u0105dku",
    /^zamknij informację o( plikach)? cookies$/,
    "zapisz i zamknij",
    // Russian (RU)
    // e.g. "понятно", "всё понятно", "хорошо", "ясно"; \w does not match Cyrillic
    /^\s*(хорошо|ясно|(вс[её]\s+)?(понятно|понял[аи]?))[!.]*\s*$/is,
    // e.g. "закрыть", "закрыть уведомление о cookie"
    /^\s*закрыть([\s-]+(это|эту|баннер|уведомлени[а-яё]*|окно|сообщени[а-яё]*|плашк[а-яё]*|информаци[а-яё]*)){0,2}([\s-]+(о|об)[\s-]+(cookie|куки[а-яё]*|файл[а-яё]*[\s-]+cookie))?\s*$/is,
    // e.g. "больше не показывать", "не показывать снова"
    /^\s*(больше\s+не\s+показывать|не\s+показывать(\s+(снова|больше|это\s+сообщение))?)\s*$/is,
    // e.g. "подтвердить", "подтверждаю выбор", "сохранить настройки", "сохранить и закрыть"
    /^\s*(подтвер(дить|ждаю|ждени[а-яё]*)|сохран(ить|яю|ение))([\s-]+(мой|мои|моё|свой|свои|своё)?[\s-]*(выбор[а-яё]*|настройк[а-яё]*|парамет[а-яё]*|предпочтени[а-яё]*|согласи[а-яё]*))?([\s-]+и[\s-]+(закрыть|продолжить))?\s*$/is,
    "\u043F\u0440\u043E\u0434\u043E\u043B\u0436\u0438\u0442\u044C"
  ];

  // lib/heuristics.ts
  var BUTTON_LIKE_ELEMENT_SELECTOR = 'button, input[type="button"], input[type="submit"], a, [role="button"], [class*="button"]';
  var TEXT_LIMIT = 1e5;
  var POPUP_SEARCH_MAX_TIME = 100;
  function checkHeuristicPatterns(allText, detectPatterns = DETECT_PATTERNS) {
    allText = allText.slice(0, TEXT_LIMIT);
    const patterns = [];
    const snippets2 = [];
    for (const p of detectPatterns) {
      const matches = allText?.match(p);
      if (matches) {
        patterns.push(p.toString());
        for (const m of matches) {
          if (typeof m === "string") {
            snippets2.push(m.substring(0, 200));
          }
        }
      }
    }
    return { patterns, snippets: snippets2 };
  }
  function getActionablePopups(mode = "reject", timeout = POPUP_SEARCH_MAX_TIME) {
    const acceptedLevels = mode === "tier2" ? ["reject", "tier1", "tier2"] : mode === "tier1" ? ["reject", "tier1"] : ["reject"];
    if (acceptedLevels.length === 0) {
      return [];
    }
    const popups = getPotentialPopups(timeout);
    const result = popups.reduce((acc, popup) => {
      const popupText = popup.text?.trim();
      if (popupText) {
        if (isExcludedPopup(popupText)) {
          return acc;
        }
        const { patterns } = checkHeuristicPatterns(popupText);
        if (patterns.length > 0) {
          classifyButtons(popup.buttons);
          popup.regexClassification = classifyPopup(popup.buttons);
          acc.push({
            ...popup
          });
        }
      }
      return acc;
    }, []);
    return result.filter((popup) => popup.regexClassification !== void 0 && acceptedLevels.includes(popup.regexClassification)).sort((a, b) => (a.regexClassification ?? "") > (b.regexClassification ?? "") ? 1 : -1);
  }
  function classifyButtons(buttons) {
    for (const button of buttons) {
      button.regexClassification = classifyButtonTextRegex(button.text);
    }
  }
  function isExcludedPopup(popupText, excludePatterns = DETECT_NEVER_MATCH_PATTERNS) {
    if (!popupText) {
      return false;
    }
    const truncated = popupText.slice(0, TEXT_LIMIT);
    return excludePatterns.some((p) => p.test(truncated));
  }
  function classifyPopup(buttons) {
    const { reject, settings, accept, acknowledge } = buttons.reduce(
      (acc, button) => {
        if (button.regexClassification && button.regexClassification !== "other") {
          acc[button.regexClassification]++;
        }
        return acc;
      },
      { reject: 0, settings: 0, accept: 0, acknowledge: 0 }
    );
    if (reject > 0) {
      return "reject";
    }
    if (settings > 0) {
      return "none";
    }
    if (acknowledge > 0) {
      return "tier1";
    }
    if (accept > 0) {
      return accept === 1 ? "tier2" : "none";
    }
    return "none";
  }
  function testButtonMatches(buttonText, matchPatterns, neverMatchPatterns) {
    if (!buttonText) {
      return false;
    }
    const cleanedButtonText = cleanButtonText(buttonText);
    return !neverMatchPatterns.some((p) => p instanceof RegExp && p.test(cleanedButtonText) || p === cleanedButtonText) && matchPatterns.some((p) => p instanceof RegExp && p.test(cleanedButtonText) || p === cleanedButtonText);
  }
  function cleanButtonText(buttonText) {
    let result = buttonText.toLowerCase();
    result = result.replace(/[“”"'/#&[\]→✕×⟩❯><✗×‘’›«»]+/g, "");
    result = result.replace(
      /[\u{1F600}-\u{1F64F}\u{1F300}-\u{1F5FF}\u{1F680}-\u{1F6FF}\u{1F1E0}-\u{1F1FF}\u2600-\u26FF\u2700-\u27BF\u{1F900}-\u{1F9FF}\u{1FA70}-\u{1FAFF}]/gu,
      ""
    );
    result = result.replace(/\n+/g, " ");
    result = result.replace(/\s+/g, " ");
    result = result.trim();
    return result;
  }
  function classifyButtonTextRegex(buttonText) {
    if (testButtonMatches(buttonText, REJECT_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "reject";
    }
    if (testButtonMatches(buttonText, SETTINGS_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "settings";
    }
    if (testButtonMatches(buttonText, ACCEPT_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "accept";
    }
    if (testButtonMatches(buttonText, ACKNOWLEDGE_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "acknowledge";
    }
    return "other";
  }
  function getPotentialPopups(timeout = POPUP_SEARCH_MAX_TIME) {
    const isFramed = !isTopFrame();
    if (isFramed && window.parent && window.parent !== window.top) {
      return [];
    }
    return collectPotentialPopups(isFramed, timeout);
  }
  function collectPotentialPopups(isFramed, timeout = POPUP_SEARCH_MAX_TIME) {
    let elements = [];
    if (!isFramed) {
      elements = getPopupLikeElements(timeout);
    } else {
      const doc = document.body || document.documentElement;
      if (doc && isElementVisible(doc) && doc.innerText) {
        elements.push(doc);
      }
    }
    const potentialPopups = [];
    for (const el of elements) {
      if (el.innerText) {
        potentialPopups.push({
          text: el.innerText,
          element: el,
          buttons: getButtonData(el)
        });
      }
    }
    return potentialPopups;
  }
  function isDialogLikeElement(node) {
    if (node.tagName === "DIALOG" && node.hasAttribute("open")) {
      return true;
    }
    if (node.getAttribute("role") === "dialog" || node.getAttribute("aria-modal") === "true") {
      return true;
    }
    return false;
  }
  function getPopupLikeElements(timeout = POPUP_SEARCH_MAX_TIME) {
    const startTime = performance.now();
    const walker = document.createTreeWalker(
      document.documentElement,
      NodeFilter.SHOW_ELEMENT,
      // visit only element nodes
      {
        acceptNode(node) {
          if (node.tagName === "BODY") {
            return NodeFilter.FILTER_SKIP;
          }
          if (isElementVisible(node)) {
            const cssPosition = window.getComputedStyle(node).position;
            if (cssPosition === "fixed" || cssPosition === "sticky") {
              return NodeFilter.FILTER_ACCEPT;
            }
            if (isDialogLikeElement(node)) {
              return NodeFilter.FILTER_ACCEPT;
            }
          }
          if (performance.now() - startTime > timeout) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_SKIP;
        }
      }
    );
    const found = [];
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      found.push(node);
    }
    return excludeContainers(found.filter((element) => element.innerText?.trim()));
  }
  function getButtonData(el) {
    const actionableButtons = excludeContainers(getButtonLikeElements(el)).filter(
      (b) => isElementVisible(b) && !isDisabled(b) && (b.innerText?.trim() || // <input> values do not appear in innerText
      b instanceof HTMLInputElement && ["submit", "button"].includes(b.type) && b.value?.trim())
    );
    return actionableButtons.map((b) => ({
      text: (b.innerText || b.textContent || "").trim() || b.value?.trim() || "",
      element: b
    }));
  }
  function getButtonLikeElements(el) {
    return Array.from(el.querySelectorAll(BUTTON_LIKE_ELEMENT_SELECTOR));
  }
  function isDisabled(el) {
    return "disabled" in el && Boolean(el.disabled) || el.hasAttribute("disabled");
  }
  function excludeContainers(elements) {
    const results = [];
    if (elements.length > 0) {
      for (let i = elements.length - 1; i >= 0; i--) {
        let container = false;
        for (let j = 0; j < elements.length; j++) {
          if (i !== j && elements[i].contains(elements[j])) {
            container = true;
            break;
          }
        }
        if (!container) {
          results.push(elements[i]);
        }
      }
    }
    return results;
  }

  // lib/cmps/base.ts
  var defaultRunContext = {
    main: true,
    frame: false,
    urlPattern: ""
  };
  var AutoConsentCMPBase = class {
    constructor(autoconsentInstance) {
      this.name = "BASERULE";
      this.runContext = defaultRunContext;
      this.autoconsent = autoconsentInstance;
    }
    get hasSelfTest() {
      throw new Error("Not Implemented");
    }
    get isIntermediate() {
      throw new Error("Not Implemented");
    }
    get isCosmetic() {
      throw new Error("Not Implemented");
    }
    mainWorldEval(snippetId) {
      const snippet = snippets[snippetId];
      if (!snippet) {
        this.autoconsent.config.logs.errors && console.warn("Snippet not found", snippetId);
        return Promise.resolve(false);
      }
      const logsConfig = this.autoconsent.config.logs;
      if (this.autoconsent.config.isMainWorld) {
        logsConfig.evals && console.log("inline eval:", snippetId, snippet);
        let result = false;
        try {
          result = !!snippet.call(globalThis);
        } catch (e) {
          logsConfig.evals && console.error("error evaluating rule", snippetId, e);
        }
        return Promise.resolve(result);
      }
      const snippetSrc = getFunctionBody(snippet);
      logsConfig.evals && console.log("async eval:", snippetId, snippetSrc);
      return requestEval(snippetSrc, snippetId).catch((e) => {
        logsConfig.evals && console.error("error evaluating rule", snippetId, e);
        return false;
      });
    }
    checkRunContext() {
      if (!this.checkFrameContext(isTopFrame())) {
        return false;
      }
      if (this.runContext.urlPattern && !this.hasMatchingUrlPattern()) {
        return false;
      }
      return true;
    }
    checkFrameContext(isTop) {
      const runCtx = {
        ...defaultRunContext,
        ...this.runContext
      };
      if (isTop && !runCtx.main) {
        return false;
      }
      if (!isTop && !runCtx.frame) {
        return false;
      }
      return true;
    }
    hasMatchingUrlPattern() {
      return Boolean(this.runContext?.urlPattern && window.location.href.match(this.runContext.urlPattern));
    }
    detectCmp() {
      throw new Error("Not Implemented");
    }
    async detectPopup() {
      return false;
    }
    optOut() {
      throw new Error("Not Implemented");
    }
    optIn() {
      throw new Error("Not Implemented");
    }
    openCmp() {
      throw new Error("Not Implemented");
    }
    async test() {
      return Promise.resolve(true);
    }
    async highlightElements(elements, all = false, delayTimeout = 2e3) {
      if (elements.length === 0) {
        return;
      }
      if (!all) {
        elements = [elements[0]];
      }
      this.autoconsent.sendContentMessage({
        type: "visualDelay",
        timeout: delayTimeout
      });
      for (const el of elements) {
        this.autoconsent.config.logs.rulesteps && console.log("highlighting", el);
        highlightNode(el);
      }
      await this.wait(delayTimeout);
      for (const el of elements) {
        unhighlightNode(el);
      }
    }
    // Implementing DomActionsProvider below:
    async clickElement(element) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements([element]);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.clickElement(element);
    }
    async click(selector, all = false) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements(this.elementSelector(selector), all);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.click(selector, all);
    }
    elementExists(selector) {
      return this.autoconsent.domActions.elementExists(selector);
    }
    elementVisible(selector, check) {
      return this.autoconsent.domActions.elementVisible(selector, check);
    }
    waitForElement(selector, timeout) {
      return this.autoconsent.domActions.waitForElement(selector, timeout);
    }
    waitForVisible(selector, timeout, check) {
      return this.autoconsent.domActions.waitForVisible(selector, timeout, check);
    }
    async waitForThenClick(selector, timeout, all, retries, retryInterval) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements(this.elementSelector(selector), all);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.waitForThenClick(selector, timeout, all, retries, retryInterval);
    }
    wait(ms) {
      return this.autoconsent.domActions.wait(ms);
    }
    hide(selector, method) {
      return this.autoconsent.domActions.hide(selector, method);
    }
    stylesheet(cssRule, stylesheetId) {
      return this.autoconsent.domActions.stylesheet(cssRule, stylesheetId);
    }
    removeClass(selector, className) {
      return this.autoconsent.domActions.removeClass(selector, className);
    }
    setStyle(selector, css) {
      return this.autoconsent.domActions.setStyle(selector, css);
    }
    addStyle(selector, css) {
      return this.autoconsent.domActions.addStyle(selector, css);
    }
    cookieContains(substring) {
      return this.autoconsent.domActions.cookieContains(substring);
    }
    prehide(selector) {
      return this.autoconsent.domActions.prehide(selector);
    }
    undoPrehide() {
      return this.autoconsent.domActions.undoPrehide();
    }
    querySingleReplySelector(selector, parent) {
      return this.autoconsent.domActions.querySingleReplySelector(selector, parent);
    }
    querySelectorChain(selectors) {
      return this.autoconsent.domActions.querySelectorChain(selectors);
    }
    elementSelector(selector) {
      return this.autoconsent.domActions.elementSelector(selector);
    }
    waitForMutation(selector) {
      return this.autoconsent.domActions.waitForMutation(selector);
    }
  };
  var AutoConsentCMP = class extends AutoConsentCMPBase {
    constructor(rule, autoconsentInstance) {
      super(autoconsentInstance);
      this.rule = rule;
      this.name = rule.name;
      this.runContext = rule.runContext || defaultRunContext;
    }
    get hasSelfTest() {
      return !!this.rule.test && this.rule.test.length > 0;
    }
    get isIntermediate() {
      return !!this.rule.intermediate;
    }
    get isCosmetic() {
      return !!this.rule.cosmetic;
    }
    get prehideSelectors() {
      return this.rule.prehideSelectors || [];
    }
    async detectCmp() {
      if (this.rule.detectCmp) {
        return this._runRulesSequentially(this.rule.detectCmp, this.autoconsent.config.logs.detectionsteps);
      }
      return false;
    }
    async detectPopup() {
      if (this.rule.detectPopup) {
        return this._runRulesSequentially(this.rule.detectPopup, this.autoconsent.config.logs.detectionsteps);
      }
      return false;
    }
    async optOut() {
      const logsConfig = this.autoconsent.config.logs;
      if (this.rule.optOut) {
        logsConfig.lifecycle && console.log("Initiated optOut()", this.rule.optOut);
        return this._runRulesSequentially(this.rule.optOut, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async optIn() {
      const logsConfig = this.autoconsent.config.logs;
      if (this.rule.optIn) {
        logsConfig.lifecycle && console.log("Initiated optIn()", this.rule.optIn);
        return this._runRulesSequentially(this.rule.optIn, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async openCmp() {
      if (this.rule.openCmp) {
        return this._runRulesSequentially(this.rule.openCmp, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async test() {
      if (this.hasSelfTest && this.rule.test) {
        return this._runRulesSequentially(this.rule.test, this.autoconsent.config.logs.rulesteps);
      }
      return super.test();
    }
    async evaluateRuleStep(rule) {
      const results = [];
      const logsConfig = this.autoconsent.config.logs;
      if (rule.exists) {
        results.push(this.elementExists(rule.exists));
      }
      if (rule.visible) {
        results.push(this.elementVisible(rule.visible, rule.check));
      }
      if (rule.eval) {
        const res = this.mainWorldEval(rule.eval);
        results.push(res);
      }
      if (rule.waitFor) {
        results.push(this.waitForElement(rule.waitFor, rule.timeout));
      }
      if (rule.waitForVisible) {
        results.push(this.waitForVisible(rule.waitForVisible, rule.timeout, rule.check));
      }
      if (rule.click) {
        results.push(this.click(rule.click, rule.all));
      }
      if (rule.waitForThenClick) {
        results.push(this.waitForThenClick(rule.waitForThenClick, rule.timeout, rule.all, rule.retry, rule.retryInterval));
      }
      if (rule.wait) {
        results.push(this.wait(rule.wait));
      }
      if (rule.hide) {
        results.push(this.hide(rule.hide, rule.method));
      }
      if (rule.stylesheet !== void 0) {
        results.push(this.stylesheet(rule.stylesheet, rule.stylesheetId));
      }
      if (rule.removeClass !== void 0) {
        results.push(rule.selector ? this.removeClass(rule.selector, rule.removeClass) : false);
      }
      if (rule.setStyle !== void 0) {
        results.push(rule.selector ? this.setStyle(rule.selector, rule.setStyle) : false);
      }
      if (rule.addStyle !== void 0) {
        results.push(rule.selector ? this.addStyle(rule.selector, rule.addStyle) : false);
      }
      if (rule.cookieContains) {
        results.push(this.cookieContains(rule.cookieContains));
      }
      if (rule.if) {
        if (!rule.if.exists && !rule.if.visible) {
          console.error("invalid conditional rule", rule.if);
          return false;
        }
        if (!rule.then) {
          console.error('invalid conditional rule, missing "then" step', rule.if);
          return false;
        }
        const condition = await this.evaluateRuleStep(rule.if);
        logsConfig.rulesteps && console.log("Condition is", condition);
        if (condition) {
          results.push(this._runRulesSequentially(rule.then, logsConfig.rulesteps));
        } else if (rule.else) {
          results.push(this._runRulesSequentially(rule.else, logsConfig.rulesteps));
        } else {
          results.push(true);
        }
      }
      if (rule.any) {
        let resultOfAny = false;
        for (const step of rule.any) {
          if (await this.evaluateRuleStep(step)) {
            resultOfAny = true;
            break;
          }
        }
        results.push(resultOfAny);
      }
      if (results.length === 0) {
        logsConfig.errors && console.warn("Unrecognized rule", rule);
        return false;
      }
      const all = await Promise.all(results);
      const result = all.reduce((a, b) => a && b, true);
      if (rule.negated) {
        return !result;
      }
      return result;
    }
    async _runRulesParallel(rules) {
      const results = rules.map((rule) => this.evaluateRuleStep(rule));
      const detections = await Promise.all(results);
      return detections.every((r) => !!r);
    }
    async _runRulesSequentially(rules, logSteps = true) {
      for (const rule of rules) {
        logSteps && console.log("Running rule...", rule);
        const result = await this.evaluateRuleStep(rule);
        logSteps && console.log("...rule result", result);
        if (!result && !rule.optional) {
          return false;
        }
      }
      return true;
    }
  };
  var AutoConsentHeuristicCMP = class extends AutoConsentCMPBase {
    constructor(autoconsentInstance, mode = "reject") {
      super(autoconsentInstance);
      this.popups = [];
      this.name = "HEURISTIC";
      this.runContext = {
        main: true,
        frame: false
        // do not run in iframes for security reasons
      };
      this.mode = mode;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      await new Promise((resolve) => setTimeout(resolve, 0));
      this.autoconsent.config.performanceLoggingEnabled && performance.mark("heuristicDetectorStart");
      this.popups = getActionablePopups(this.mode, this.autoconsent.config.heuristicPopupSearchTimeout);
      this.autoconsent.config.performanceLoggingEnabled && performance.mark("heuristicDetectorEnd");
      this.autoconsent.config.performanceLoggingEnabled && performance.measure("heuristicDetector", "heuristicDetectorStart", "heuristicDetectorEnd");
      if (this.popups.length > 0) {
        this.name = `HEURISTIC-${this.popups[0].regexClassification?.toUpperCase()}`;
        return Promise.resolve(true);
      }
      return Promise.resolve(false);
    }
    async detectPopup() {
      if (this.popups.length > 0) {
        if (this.popups.length > 1) {
          this.autoconsent.config.logs.errors && console.warn("Heuristic found multiple popups");
        }
        return true;
      }
      return false;
    }
    getTargetButton() {
      const popup = this.popups[0];
      const level = popup.regexClassification;
      const buttons = popup.buttons;
      const targetButtonType = level === "reject" ? "reject" : level === "tier1" ? "acknowledge" : "accept";
      return buttons.find((button) => button.regexClassification === targetButtonType);
    }
    optOut() {
      const button = this.getTargetButton();
      if (button) {
        return this.clickElement(button.element);
      }
      return Promise.resolve(false);
    }
    optIn() {
      throw new Error("Not Implemented");
    }
    openCmp() {
      throw new Error("Not Implemented");
    }
    async test() {
      const button = this.getTargetButton();
      if (button) {
        await this.wait(500);
        return !isElementVisible(button.element);
      }
      return false;
    }
  };

  // lib/cmps/trustarc-top.ts
  var cookieSettingsButton = "#truste-show-consent";
  var shortcutOptOut = "#truste-consent-required";
  var shortcutOptIn = "#truste-consent-button";
  var popupContent = "#truste-consent-content";
  var bannerOverlay = "#trustarc-banner-overlay";
  var bannerContainer = "#truste-consent-track";
  var TrustArcTop = class extends AutoConsentCMPBase {
    constructor(autoconsentInstance) {
      super(autoconsentInstance);
      this.name = "TrustArc-top";
      this.prehideSelectors = [".trustarc-banner-container", `.truste_popframe,.truste_overlay,.truste_box_overlay,${bannerContainer}`];
      this.runContext = {
        main: true,
        frame: false
      };
      this._shortcutButton = null;
      this._optInDone = false;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      if (this._optInDone) {
        return false;
      }
      return !this._shortcutButton;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      const result = this.elementExists(`${cookieSettingsButton},${bannerContainer}`);
      if (result) {
        this._shortcutButton = document.querySelector(shortcutOptOut);
      }
      return result;
    }
    async detectPopup() {
      return this.elementVisible(`${popupContent},${bannerOverlay},${bannerContainer}`, "any");
    }
    async optOut() {
      if (this.elementExists(shortcutOptOut)) {
        this.click(shortcutOptOut);
        return true;
      }
      hideElements(getStyleElement(), `.truste_popframe, .truste_overlay, .truste_box_overlay, ${bannerContainer}`);
      await this.click(cookieSettingsButton);
      setTimeout(() => {
        getStyleElement().remove();
      }, 1e4);
      return true;
    }
    async optIn() {
      this._optInDone = true;
      return await this.click(shortcutOptIn);
    }
    async openCmp() {
      return true;
    }
    async test() {
      await this.wait(500);
      return await this.mainWorldEval("EVAL_TRUSTARC_TOP");
    }
  };

  // lib/cmps/cookiebot.ts
  var Cookiebot = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Cybotcookiebot";
      this.prehideSelectors = [
        "#CybotCookiebotDialog,#CybotCookiebotDialogBodyUnderlay,#dtcookie-container,#cookiebanner,#cb-cookieoverlay,.modal--cookie-banner,#cookiebanner_outer,#CookieBanner"
      ];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return await this.mainWorldEval("EVAL_COOKIEBOT_1");
    }
    async detectPopup() {
      return this.mainWorldEval("EVAL_COOKIEBOT_2");
    }
    async optOut() {
      if (this.elementVisible("#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll")) {
        return await this.click("#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll");
      }
      await this.wait(500);
      let res = await this.mainWorldEval("EVAL_COOKIEBOT_3");
      await this.wait(1e3);
      res = res && await this.mainWorldEval("EVAL_COOKIEBOT_4");
      if (this.elementVisible("#CybotCookiebotDialogBodyButtonDecline")) {
        return await this.click("#CybotCookiebotDialogBodyButtonDecline");
      }
      return res;
    }
    async optIn() {
      if (this.elementExists("#dtcookie-container")) {
        return await this.click(".h-dtcookie-accept");
      }
      await this.click(".CybotCookiebotDialogBodyLevelButton:not(:checked):enabled", true);
      await this.click("#CybotCookiebotDialogBodyLevelButtonAccept");
      await this.click("#CybotCookiebotDialogBodyButtonAccept");
      return true;
    }
    async test() {
      await this.wait(500);
      return await this.mainWorldEval("EVAL_COOKIEBOT_5");
    }
  };

  // lib/cmps/sourcepoint-frame.ts
  var SourcePoint = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Sourcepoint-frame";
      this.prehideSelectors = ["div[id^='sp_message_container_'],.message-overlay", "#sp_privacy_manager_container"];
      this.ccpaNotice = false;
      this.ccpaPopup = false;
      this.runContext = {
        main: true,
        frame: true
      };
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      const url = new URL(location.href);
      if (url.searchParams.has("message_id") && url.hostname === "ccpa-notice.sp-prod.net") {
        this.ccpaNotice = true;
        return true;
      }
      if (url.hostname === "ccpa-pm.sp-prod.net") {
        this.ccpaPopup = true;
        return true;
      }
      return (url.pathname === "/index.html" || url.pathname === "/privacy-manager/index.html" || url.pathname === "/ccpa_pm/index.html" || url.pathname === "/us_pm/index.html") && (url.searchParams.has("message_id") || url.searchParams.has("requestUUID") || url.searchParams.has("consentUUID"));
    }
    async detectPopup() {
      if (this.ccpaNotice) {
        return true;
      }
      if (this.ccpaPopup) {
        return await this.waitForElement(".priv-save-btn", 2e3);
      }
      await this.waitForElement(
        ".sp_choice_type_11,.sp_choice_type_12,.sp_choice_type_13,.sp_choice_type_ACCEPT_ALL,.sp_choice_type_SAVE_AND_EXIT",
        2e3
      );
      return !this.elementExists(".sp_choice_type_9");
    }
    async optIn() {
      await this.waitForElement(".sp_choice_type_11,.sp_choice_type_ACCEPT_ALL", 2e3);
      if (await this.click(".sp_choice_type_11")) {
        return true;
      }
      if (await this.click(".sp_choice_type_ACCEPT_ALL")) {
        return true;
      }
      return false;
    }
    isManagerOpen() {
      if (location.pathname === "/privacy-manager/index.html" || location.pathname === "/ccpa_pm/index.html") {
        return true;
      }
      if (location.pathname === "/us_pm/index.html" && !document.querySelector(".sp_choice_type_11,.sp_choice_type_ACCEPT_ALL")) {
        return true;
      }
      return false;
    }
    async optOut() {
      await this.wait(500);
      const logsConfig = this.autoconsent.config.logs;
      if (this.ccpaPopup) {
        const toggles = document.querySelectorAll(
          ".priv-purpose-container .sp-switch-arrow-block a.neutral.on .right"
        );
        for (const t of toggles) {
          t.click();
        }
        const switches = document.querySelectorAll(
          ".priv-purpose-container .sp-switch-arrow-block a.switch-bg.on"
        );
        for (const t of switches) {
          t.click();
        }
        return await this.click(".priv-save-btn");
      }
      if (this.elementVisible(".sp_choice_type_SE", "any")) {
        await this.click(
          [
            "xpath///div[contains(., 'Do not share my personal information') and contains(@class, 'switch-container')]",
            ".pm-switch[aria-checked=false] .slider"
          ],
          false
        );
        return await this.click(".sp_choice_type_SE");
      }
      if (!this.isManagerOpen()) {
        const manageSelector = '.sp_choice_type_12,[data-choice]:not([class*="sp_choice_type_"])';
        const actionable = await this.waitForVisible(`${manageSelector},.sp_choice_type_13`);
        if (!actionable) {
          return false;
        }
        if (this.elementVisible(".sp_choice_type_13", "any")) {
          return await this.click(".sp_choice_type_13");
        }
        await this.click(manageSelector);
        await waitFor(() => this.isManagerOpen(), 200, 100);
      }
      await this.waitForElement(".type-modal", 2e4);
      if (this.elementExists("[role=tablist]")) {
        await this.waitForElement("[role=tablist] [role=tab]", 1e4);
      }
      this.waitForThenClick(".ccpa-stack .pm-switch[aria-checked=true] .slider", 500, true);
      try {
        const rejectSelector1 = ".sp_choice_type_REJECT_ALL";
        const rejectSelector2 = ".reject-toggle";
        const path = await Promise.race([
          this.waitForElement(rejectSelector1, 2e3).then((success) => success ? 0 : -1),
          this.waitForElement(rejectSelector2, 2e3).then((success) => success ? 1 : -1),
          this.waitForElement(".pm-features", 2e3).then((success) => success ? 2 : -1)
        ]);
        if (path === 0) {
          await this.waitForVisible(rejectSelector1);
          return await this.click(rejectSelector1);
        } else if (path === 1) {
          await this.click(rejectSelector2);
        } else if (path === 2) {
          await this.waitForElement(".pm-features", 1e4);
          await this.click(".checked > span", true);
          await this.click(".chevron");
        }
      } catch (e) {
        logsConfig.errors && console.warn(e);
      }
      return await this.click(".sp_choice_type_SAVE_AND_EXIT");
    }
  };

  // lib/cmps/consentmanager.ts
  var ConsentManager = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "consentmanager.net";
      this.prehideSelectors = ["#cmpbox,#cmpbox2"];
      this.apiAvailable = false;
    }
    get hasSelfTest() {
      return this.apiAvailable;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      this.apiAvailable = await this.mainWorldEval("EVAL_CONSENTMANAGER_1");
      if (!this.apiAvailable) {
        return this.elementExists("#cmpbox");
      } else {
        return true;
      }
    }
    async detectPopup() {
      if (this.elementVisible("#cmpbox .cmpmore", "any")) {
        return true;
      } else if (this.apiAvailable) {
        await this.wait(500);
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_2");
      }
      return false;
    }
    async optOut() {
      await this.wait(500);
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_3");
      }
      if (await this.click(".cmpboxbtnno")) {
        return true;
      }
      if (this.elementExists(".cmpwelcomeprpsbtn")) {
        await this.click(".cmpwelcomeprpsbtn > a[aria-checked=true]", true);
        await this.click(".cmpboxbtnsave");
        return true;
      }
      await this.click(".cmpboxbtncustom");
      await this.waitForElement(".cmptblbox", 2e3);
      await this.click(".cmptdchoice > a[aria-checked=true]", true);
      await this.click(".cmpboxbtnyescustomchoices");
      this.hide("#cmpwrapper,#cmpbox", "display");
      return true;
    }
    async optIn() {
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_4");
      }
      return await this.click(".cmpboxbtnyes");
    }
    async test() {
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_5");
      }
      return false;
    }
  };

  // lib/cmps/evidon.ts
  var Evidon = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Evidon";
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists("#_evidon_banner");
    }
    async detectPopup() {
      return this.elementVisible("#_evidon_banner", "any");
    }
    async optOut() {
      if (await this.click("#_evidon-decline-button")) {
        return true;
      }
      hideElements(getStyleElement(), "#evidon-prefdiag-overlay,#evidon-prefdiag-background,#_evidon-background");
      await this.waitForThenClick("#_evidon-option-button");
      await this.waitForElement("#evidon-prefdiag-overlay", 5e3);
      await this.wait(500);
      await this.waitForThenClick("#evidon-prefdiag-decline");
      return true;
    }
    async optIn() {
      return await this.click("#_evidon-accept-button");
    }
  };

  // lib/cmps/onetrust.ts
  var Onetrust = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Onetrust";
      this.prehideSelectors = ["#onetrust-banner-sdk,#onetrust-consent-sdk,.onetrust-pc-dark-filter,.js-consent-banner"];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists("#onetrust-banner-sdk") || this.elementVisible("#onetrust-pc-sdk", "any");
    }
    async detectPopup() {
      return this.elementVisible("#onetrust-banner-sdk,#onetrust-pc-sdk", "any");
    }
    async optOut() {
      await this.wait(500);
      if (this.elementVisible("#onetrust-reject-all-handler", "any")) {
        return await this.click("#onetrust-reject-all-handler");
      }
      if (this.elementVisible(".ot-pc-refuse-all-handler", "any")) {
        return await this.click(".ot-pc-refuse-all-handler");
      }
      if (this.elementVisible(".js-reject-cookies", "any")) {
        return await this.click(".js-reject-cookies");
      }
      if (this.elementVisible(".onetrust-close-btn-handler", "any")) {
        const closeBtn = document.querySelector(".onetrust-close-btn-handler");
        const btnText = closeBtn?.textContent?.toLowerCase() || "";
        const rejectPatterns = ["without", "ohne", "sans", "sin ", "zonder", "senza", "refuse", "decline", "reject", "ablehnen"];
        if (rejectPatterns.some((pattern) => btnText.includes(pattern))) {
          return await this.click(".onetrust-close-btn-handler");
        }
        const banner = document.getElementById("onetrust-banner-sdk");
        const isCloseOnlyNotice = banner?.classList.contains("ot-close-btn-link") && !this.elementExists(
          "#onetrust-accept-btn-handler,#onetrust-reject-all-handler,#onetrust-pc-btn-handler,.ot-sdk-show-settings,button.js-cookie-settings"
        );
        if (isCloseOnlyNotice) {
          return await this.click(".onetrust-close-btn-handler");
        }
      }
      if (this.elementExists("#onetrust-pc-btn-handler")) {
        await this.click("#onetrust-pc-btn-handler");
      } else {
        await this.click(".ot-sdk-show-settings,button.js-cookie-settings");
      }
      await this.waitForElement("#onetrust-consent-sdk", 2e3);
      await this.wait(1e3);
      await this.click("#onetrust-consent-sdk input.category-switch-handler:checked,.js-editor-toggle-state:checked", true);
      await this.wait(1e3);
      await this.waitForElement(".save-preference-btn-handler,.js-consent-save", 2e3);
      await this.click(".save-preference-btn-handler,.js-consent-save");
      await this.waitForVisible("#onetrust-banner-sdk", 5e3, "none");
      return true;
    }
    async optIn() {
      return await this.click("#onetrust-accept-btn-handler,#accept-recommended-btn-handler,.js-accept-cookies");
    }
    async test() {
      return await waitFor(() => this.mainWorldEval("EVAL_ONETRUST_1"), 10, 500);
    }
  };

  // lib/cmps/klaro.ts
  var Klaro = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Klaro";
      this.prehideSelectors = [".klaro"];
      this.settingsOpen = false;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      if (this.elementExists(".klaro > .cookie-modal")) {
        this.settingsOpen = true;
        return true;
      }
      return this.elementExists(".klaro > .cookie-notice");
    }
    async detectPopup() {
      return this.elementVisible(".klaro > .cookie-notice,.klaro > .cookie-modal", "any");
    }
    async optOut() {
      const apiOptOutSuccess = await this.mainWorldEval("EVAL_KLARO_TRY_API_OPT_OUT");
      if (apiOptOutSuccess) {
        return true;
      }
      if (await this.click(".klaro .cn-decline")) {
        return true;
      }
      await this.mainWorldEval("EVAL_KLARO_OPEN_POPUP");
      if (await this.click(".klaro .cn-decline")) {
        return true;
      }
      await this.click(
        ".cm-purpose:not(.cm-toggle-all) > input:not(.half-checked,.required,.only-required),.cm-purpose:not(.cm-toggle-all) > div > input:not(.half-checked,.required,.only-required)",
        true
      );
      return await this.click(".cm-btn-accept,.cm-button");
    }
    async optIn() {
      if (await this.click(".klaro .cm-btn-accept-all")) {
        return true;
      }
      if (this.settingsOpen) {
        await this.click(".cm-purpose:not(.cm-toggle-all) > input.half-checked", true);
        return await this.click(".cm-btn-accept");
      }
      return await this.click(".klaro .cookie-notice .cm-btn-success");
    }
    async test() {
      return await this.mainWorldEval("EVAL_KLARO_1");
    }
  };

  // lib/cmps/uniconsent.ts
  var Uniconsent = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Uniconsent";
    }
    get prehideSelectors() {
      return [".unic", ".modal:has(.unic)"];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists(".unic .unic-box,.unic .unic-bar,.unic .unic-modal");
    }
    async detectPopup() {
      return this.elementVisible(".unic .unic-box,.unic .unic-bar,.unic .unic-modal", "any");
    }
    async optOut() {
      await this.waitForElement(".unic button", 1e3);
      document.querySelectorAll(".unic button").forEach((button) => {
        const text = button.textContent || "";
        if (text.includes("Manage Options") || text.includes("Optionen verwalten")) {
          button.click();
        }
      });
      if (await this.waitForElement(".unic input[type=checkbox]", 1e3)) {
        await this.waitForElement(".unic button", 1e3);
        document.querySelectorAll(".unic input[type=checkbox]").forEach((c) => {
          if (c.checked) {
            c.click();
          }
        });
        for (const b of document.querySelectorAll(".unic button")) {
          const text = b.textContent || "";
          for (const pattern of ["Confirm Choices", "Save Choices", "Auswahl speichern"]) {
            if (text.includes(pattern)) {
              b.click();
              await this.wait(500);
              return true;
            }
          }
        }
      }
      return false;
    }
    async optIn() {
      return this.waitForThenClick(".unic #unic-agree");
    }
    async test() {
      await this.wait(1e3);
      const res = this.elementExists(".unic .unic-box,.unic .unic-bar");
      return !res;
    }
  };

  // lib/cmps/conversant.ts
  var Conversant = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.prehideSelectors = [".cmp-root"];
      this.name = "Conversant";
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists(".cmp-root .cmp-receptacle");
    }
    async detectPopup() {
      return this.elementVisible(".cmp-root .cmp-receptacle", "any");
    }
    async optOut() {
      if (!await this.waitForThenClick(".cmp-main-button:not(.cmp-main-button--primary)")) {
        return false;
      }
      if (!await this.waitForElement(".cmp-view-tab-tabs")) {
        return false;
      }
      await this.waitForThenClick(".cmp-view-tab-tabs > :first-child");
      await this.waitForThenClick(".cmp-view-tab-tabs > .cmp-view-tab--active:first-child");
      for (const item of Array.from(document.querySelectorAll(".cmp-accordion-item"))) {
        item.querySelector(".cmp-accordion-item-title").click();
        await waitFor(() => !!item.querySelector(".cmp-accordion-item-content.cmp-active"), 10, 50);
        const content = item.querySelector(".cmp-accordion-item-content.cmp-active");
        if (!content) {
          return false;
        }
        content.querySelectorAll(".cmp-toggle-actions .cmp-toggle-deny:not(.cmp-toggle-deny--active)").forEach((e) => e.click());
        content.querySelectorAll(".cmp-toggle-actions .cmp-toggle-checkbox:not(.cmp-toggle-checkbox--active)").forEach((e) => e.click());
      }
      await this.click(".cmp-main-button:not(.cmp-main-button--primary)");
      return true;
    }
    async optIn() {
      return this.waitForThenClick(".cmp-main-button.cmp-main-button--primary");
    }
    async test() {
      return document.cookie.includes("cmp-data=0");
    }
  };

  // lib/cmps/tiktok.ts
  var Tiktok = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "tiktok.com";
      this.runContext = {
        urlPattern: "tiktok"
      };
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    getShadowRoot() {
      const container = document.querySelector("tiktok-cookie-banner");
      if (!container) {
        return null;
      }
      return container.shadowRoot;
    }
    async detectCmp() {
      return this.elementExists("tiktok-cookie-banner");
    }
    async detectPopup() {
      const banner = this.getShadowRoot()?.querySelector(".tiktok-cookie-banner");
      return isElementVisible(banner);
    }
    async optOut() {
      const logsConfig = this.autoconsent.config.logs;
      const declineButton = this.getShadowRoot()?.querySelector(".button-wrapper button:first-child");
      if (declineButton) {
        logsConfig.rulesteps && console.log("[clicking]", declineButton);
        declineButton.click();
        return true;
      } else {
        logsConfig.errors && console.log("no decline button found");
        return false;
      }
    }
    async optIn() {
      const logsConfig = this.autoconsent.config.logs;
      const acceptButton = this.getShadowRoot()?.querySelector(".button-wrapper button:last-child");
      if (acceptButton) {
        logsConfig.rulesteps && console.log("[clicking]", acceptButton);
        acceptButton.click();
        return true;
      } else {
        logsConfig.errors && console.log("no accept button found");
        return false;
      }
    }
    async test() {
      const match = document.cookie.match(/cookie-consent=([^;]+)/);
      if (!match) {
        return false;
      }
      const value = JSON.parse(decodeURIComponent(match[1]));
      return Object.values(value).every((x) => typeof x !== "boolean" || x === false);
    }
  };

  // lib/cmps/admiral.ts
  var Admiral = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Admiral";
      this.consentCardSelector = "div > div[class*=Card] > div[class*=Frame] > div[class*=Pills] > button[class*=Pills__StyledPill]";
    }
    getVrmNotice() {
      return Array.from(document.querySelectorAll("body > div")).find((element) => {
        const text = element.innerText || "";
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return style.position === "fixed" && rect.width > 0 && rect.height > 0 && text.includes("Your Privacy") && text.includes("VRM") && text.includes("Admiral");
      });
    }
    isVisibleElement(element) {
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
    }
    getVrmCloseButton() {
      const notice = this.getVrmNotice();
      const closeButton = notice?.querySelector('button[aria-label="Close"]');
      return closeButton && this.isVisibleElement(closeButton) ? closeButton : null;
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return await this.elementExists(this.consentCardSelector) || Boolean(this.getVrmNotice());
    }
    async detectPopup() {
      return await this.elementVisible(this.consentCardSelector, "any") || Boolean(this.getVrmNotice());
    }
    async optOut() {
      const rejectAllSelector = "xpath///button[contains(., 'Afvis alle') or contains(., 'Reject all') or contains(., 'Odbaci sve') or contains(., 'Rechazar todo') or contains(., 'Atmesti visus') or contains(., 'Odm\xEDtnout v\u0161e') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03CC\u03BB\u03C9\u03BD') or contains(., 'Rejeitar tudo') or contains(., 'T\xFCm\xFCn\xFC reddet') or contains(., '\u041E\u0442\u043A\u043B\u043E\u043D\u0438\u0442\u044C \u0432\u0441\u0435') or contains(., 'Noraid\u012Bt visu') or contains(., 'Avvisa alla') or contains(., 'Odrzu\u0107 wszystkie') or contains(., 'Alles afwijzen') or contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043D\u0435 \u043D\u0430 \u0432\u0441\u0438\u0447\u043A\u0438') or contains(., 'Rifiuta tutto') or contains(., 'Zavrni vse') or contains(., 'Az \xF6sszes elutas\xEDt\xE1sa') or contains(., 'Respinge\u021Bi tot') or contains(., 'Alles ablehnen') or contains(., 'Tout rejeter') or contains(., 'Odmietnu\u0165 v\u0161etko') or contains(., 'L\xFCkka k\xF5ik tagasi') or contains(., 'Hylk\xE4\xE4 kaikki')]";
      if (await this.waitForElement(rejectAllSelector, 500)) {
        return await this.click(rejectAllSelector);
      }
      const vrmCloseButton = this.getVrmCloseButton();
      if (vrmCloseButton) {
        return await this.clickElement(vrmCloseButton);
      }
      const purposesButtonSelector = "xpath///button[contains(., 'Zwecke') or contains(., '\u03A3\u03BA\u03BF\u03C0\u03BF\u03AF') or contains(., 'Purposes') or contains(., '\u0426\u0435\u043B\u0438') or contains(., 'Eesm\xE4rgid') or contains(., 'Tikslai') or contains(., 'Svrhe') or contains(., 'Cele') or contains(., '\xDA\u010Dely') or contains(., 'Finalidades') or contains(., 'M\u0113r\u0137i') or contains(., 'Scopuri') or contains(., 'Fines') or contains(., '\xC4ndam\xE5l') or contains(., 'Finalit\xE9s') or contains(., 'Doeleinden') or contains(., 'Tarkoitukset') or contains(., 'Scopi') or contains(., 'Ama\xE7lar') or contains(., 'Nameni') or contains(., 'C\xE9lok') or contains(., 'Form\xE5l')]";
      const saveAndExitSelector = "xpath///button[contains(., 'Spara & avsluta') or contains(., 'Save & exit') or contains(., 'Ulo\u017Eit a ukon\u010Dit') or contains(., 'Enregistrer et quitter') or contains(., 'Speichern & Verlassen') or contains(., 'Tallenna ja poistu') or contains(., 'I\u0161saugoti ir i\u0161eiti') or contains(., 'Opslaan & afsluiten') or contains(., 'Guardar y salir') or contains(., 'Shrani in zapri') or contains(., 'Ulo\u017Ei\u0165 a ukon\u010Di\u0165') or contains(., 'Kaydet ve \xE7\u0131k\u0131\u015F yap') or contains(., '\u0421\u043E\u0445\u0440\u0430\u043D\u0438\u0442\u044C \u0438 \u0432\u044B\u0439\u0442\u0438') or contains(., 'Salvesta ja v\xE4lju') or contains(., 'Salva ed esci') or contains(., 'Gem & afslut') or contains(., '\u0391\u03C0\u03BF\u03B8\u03AE\u03BA\u03B5\u03C5\u03C3\u03B7 \u03BA\u03B1\u03B9 \u03AD\u03BE\u03BF\u03B4\u03BF\u03C2') or contains(., 'Saglab\u0101t un iziet') or contains(., 'Ment\xE9s \xE9s kil\xE9p\xE9s') or contains(., 'Guardar e sair') or contains(., 'Zapisz & zako\u0144cz') or contains(., 'Salvare \u0219i ie\u0219ire') or contains(., 'Spremi i iza\u0111i') or contains(., '\u0417\u0430\u043F\u0430\u0437\u0432\u0430\u043D\u0435 \u0438 \u0438\u0437\u0445\u043E\u0434')]";
      if (await this.waitForThenClick(purposesButtonSelector) && await this.waitForVisible(saveAndExitSelector)) {
        const popupBody = this.elementSelector(saveAndExitSelector)[0].parentElement?.parentElement;
        const checkboxes = popupBody?.querySelectorAll("input[type=checkbox]:checked");
        checkboxes?.forEach((checkbox) => checkbox.click());
        return await this.click(saveAndExitSelector);
      }
      return false;
    }
    async optIn() {
      return await this.click(
        "xpath///button[contains(., 'Sprejmi vse') or contains(., 'Prihvati sve') or contains(., 'Godk\xE4nn alla') or contains(., 'Prija\u0165 v\u0161etko') or contains(., '\u041F\u0440\u0438\u043D\u044F\u0442\u044C \u0432\u0441\u0435') or contains(., 'Aceptar todo') or contains(., '\u0391\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE \u03CC\u03BB\u03C9\u03BD') or contains(., 'Zaakceptuj wszystkie') or contains(., 'Accetta tutto') or contains(., 'Priimti visus') or contains(., 'Pie\u0146emt visu') or contains(., 'T\xFCm\xFCn\xFC kabul et') or contains(., 'Az \xF6sszes elfogad\xE1sa') or contains(., 'Accept all') or contains(., '\u041F\u0440\u0438\u0435\u043C\u0430\u043D\u0435 \u043D\u0430 \u0432\u0441\u0438\u0447\u043A\u0438') or contains(., 'Accepter alle') or contains(., 'Hyv\xE4ksy kaikki') or contains(., 'Tout accepter') or contains(., 'Alles accepteren') or contains(., 'Aktsepteeri k\xF5ik') or contains(., 'P\u0159ijmout v\u0161e') or contains(., 'Alles akzeptieren') or contains(., 'Aceitar tudo') or contains(., 'Accepta\u021Bi tot')]"
      );
    }
  };

  // lib/cmps/all.ts
  var dynamicCMPs = [
    TrustArcTop,
    Cookiebot,
    SourcePoint,
    ConsentManager,
    Evidon,
    Onetrust,
    Klaro,
    Uniconsent,
    Conversant,
    Tiktok,
    Admiral
  ];

  // lib/dom-actions.ts
  var DEFAULT_CLICK_RETRY_INTERVAL = 300;
  var CLICK_RETRY_POLL_INTERVAL = 50;
  var DomActions = class {
    constructor(autoconsentInstance) {
      this.autoconsentInstance = autoconsentInstance;
    }
    async clickElement(element) {
      if (!element || !(element instanceof HTMLElement)) {
        return false;
      }
      this.autoconsentInstance.config.logs.rulesteps && console.log("[clickElement]", element);
      element.click();
      return true;
    }
    async click(selector, all = false) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[click]", selector, all, elements);
      if (elements.length > 0) {
        if (all) {
          elements.forEach((e) => e.click());
        } else {
          elements[0].click();
        }
      }
      return elements.length > 0;
    }
    elementExists(selector) {
      const exists = this.elementSelector(selector).length > 0;
      return exists;
    }
    elementVisible(selector, check = "all") {
      const elem = this.elementSelector(selector);
      const results = new Array(elem.length);
      elem.forEach((e, i) => {
        results[i] = isElementVisible(e);
      });
      if (check === "none") {
        return results.every((r) => !r);
      } else if (results.length === 0) {
        return false;
      } else if (check === "any") {
        return results.some((r) => r);
      }
      return results.every((r) => r);
    }
    waitForElement(selector, timeout = 1e4) {
      const interval = 200;
      const times = Math.ceil(timeout / interval);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForElement]", selector);
      return waitFor(() => this.elementSelector(selector).length > 0, times, interval);
    }
    waitForVisible(selector, timeout = 1e4, check = "any") {
      const interval = 200;
      const times = Math.ceil(timeout / interval);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForVisible]", selector);
      return waitFor(() => this.elementVisible(selector, check), times, interval);
    }
    async waitForThenClick(selector, timeout = 1e4, all = false, retries = 0, retryInterval = DEFAULT_CLICK_RETRY_INTERVAL) {
      await this.waitForElement(selector, timeout);
      let clicked = await this.click(selector, all);
      for (let attempt = 0; clicked && attempt < retries; attempt++) {
        const pollTimes = Math.ceil(retryInterval / CLICK_RETRY_POLL_INTERVAL);
        const isGone = await waitFor(() => !this.elementVisible(selector, "any"), pollTimes, CLICK_RETRY_POLL_INTERVAL);
        if (isGone) {
          break;
        }
        this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForThenClick] retrying click", selector);
        clicked = await this.click(selector, all);
      }
      return clicked;
    }
    wait(ms) {
      this.autoconsentInstance.config.logs.rulesteps && this.autoconsentInstance.config.logs.waits && console.log("[wait]", ms);
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve(true);
        }, ms);
      });
    }
    cookieContains(substring) {
      return document.cookie.includes(substring);
    }
    hide(selector, method) {
      this.autoconsentInstance.config.logs.rulesteps && console.log("[hide]", selector);
      const styleEl = getStyleElement();
      return hideElements(styleEl, selector, method);
    }
    stylesheet(cssRule, stylesheetId) {
      this.autoconsentInstance.config.logs.rulesteps && console.log("[stylesheet]", cssRule, stylesheetId);
      const styleEl = getStyleElement();
      return appendStylesheetRule(styleEl, cssRule, stylesheetId);
    }
    removeClass(selector, className) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[removeClass]", selector, className, elements);
      elements.forEach((el) => el.classList.remove(className));
      return elements.length > 0;
    }
    setStyle(selector, css) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[setStyle]", selector, css, elements);
      elements.forEach((el) => el.style.cssText = css);
      return elements.length > 0;
    }
    addStyle(selector, css) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[addStyle]", selector, css, elements);
      elements.forEach((el) => el.style.cssText += "; " + css);
      return elements.length > 0;
    }
    prehide(selector) {
      const styleEl = getStyleElement("autoconsent-prehide");
      this.autoconsentInstance.config.logs.lifecycle && console.log("[prehide]", styleEl, location.href);
      return hideElements(styleEl, selector, "opacity");
    }
    undoPrehide() {
      const existingElement = getStyleElement("autoconsent-prehide");
      this.autoconsentInstance.config.logs.lifecycle && console.log("[undoprehide]", existingElement, location.href);
      existingElement.remove();
    }
    async createOrUpdateStyleSheet(cssText, styleSheet) {
      if (!styleSheet) {
        styleSheet = new CSSStyleSheet();
      }
      styleSheet = await styleSheet.replace(cssText);
      return styleSheet;
    }
    removeStyleSheet(styleSheet) {
      if (styleSheet) {
        styleSheet.replace("");
        return true;
      }
      return false;
    }
    querySingleReplySelector(selector, parent = document) {
      if (selector.startsWith("aria/")) {
        return [];
      }
      if (selector.startsWith("xpath/")) {
        const xpath = selector.slice(6);
        const result = document.evaluate(xpath, parent, null, XPathResult.ANY_TYPE, null);
        let node = null;
        const elements = [];
        while (node = result.iterateNext()) {
          elements.push(node);
        }
        return elements;
      }
      if (selector.startsWith("text/")) {
        return [];
      }
      if (selector.startsWith("pierce/")) {
        return [];
      }
      if (parent.shadowRoot) {
        return Array.from(parent.shadowRoot.querySelectorAll(selector));
      }
      if (parent.contentDocument?.querySelectorAll) {
        return Array.from(parent.contentDocument.querySelectorAll(selector));
      }
      return Array.from(parent.querySelectorAll(selector));
    }
    querySelectorChain(selectors) {
      let parent = document;
      let matches = [];
      for (const selector of selectors) {
        matches = this.querySingleReplySelector(selector, parent);
        if (matches.length === 0) {
          return [];
        }
        parent = matches[0];
      }
      return matches;
    }
    elementSelector(selector) {
      if (typeof selector === "string") {
        return this.querySingleReplySelector(selector);
      }
      return this.querySelectorChain(selector);
    }
    waitForMutation(selector, timeout = 6e4) {
      const node = this.elementSelector(selector);
      if (node.length === 0) {
        throw new Error(`${selector} did not match any elements`);
      }
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          reject(new Error("Timed out waiting for mutation"));
          observer.disconnect();
        }, timeout);
        const observer = new MutationObserver(() => {
          clearTimeout(timer);
          observer.disconnect();
          resolve(true);
        });
        observer.observe(node[0], {
          subtree: true,
          childList: true,
          attributes: true
        });
      });
    }
  };

  // lib/encoding.ts
  var compactedRuleSteps = [
    ["exists", "e"],
    ["visible", "v"],
    ["waitForThenClick", "c"],
    ["click", "k"],
    ["waitFor", "w"],
    ["waitForVisible", "wv"],
    ["hide", "h"],
    ["cookieContains", "cc"]
  ];
  function decodeNullableBoolean(value) {
    if (value === 1) {
      return true;
    }
    if (value === 0) {
      return false;
    }
    return void 0;
  }
  function decodeRules(encoded) {
    if (encoded.v > 1) {
      throw new Error("Unsupported rule format.");
    }
    return encoded.r.filter((r) => r[0] <= SUPPORTED_RULE_STEP_VERSION).map((rule) => new CompactedCMPRule(rule, encoded.s));
  }
  var CompactedCMPRule = class {
    constructor(rule, strings) {
      this.intermediate = false;
      this.optIn = [];
      this.r = rule;
      this.s = strings;
      if (this.r[10] && this.r[10].intermediate) {
        this.intermediate = this.r[10].intermediate;
      }
    }
    _decodeRuleStep(step) {
      const clonedStep = { ...step };
      const decodeRuleStep = this._decodeRuleStep.bind(this);
      for (const [longKey, shortKey] of compactedRuleSteps) {
        if (clonedStep[shortKey] !== void 0) {
          clonedStep[longKey] = this.s[clonedStep[shortKey]];
          delete clonedStep[shortKey];
        }
      }
      if (step.if) {
        clonedStep.if = decodeRuleStep(step.if);
        clonedStep.then = step.then && step.then.map(decodeRuleStep);
        if (step.else) {
          clonedStep.else = step.else.map(decodeRuleStep);
        }
      }
      if (step.any) {
        clonedStep.any = step.any.map(decodeRuleStep);
      }
      return { ...clonedStep };
    }
    get minimumRuleStepVersion() {
      return this.r[0];
    }
    get name() {
      return this.r[1];
    }
    get cosmetic() {
      return decodeNullableBoolean(this.r[2]);
    }
    get runContext() {
      const runContext = {};
      const urlPattern = this.r[3];
      const mainFrame = this.r[4];
      const runInMainFrame = decodeNullableBoolean(Math.floor(mainFrame / 10) % 10);
      const runInSubFrame = decodeNullableBoolean(mainFrame % 10);
      if (runInMainFrame !== void 0) {
        runContext.main = runInMainFrame;
      }
      if (runInSubFrame !== void 0) {
        runContext.frame = runInSubFrame;
      }
      if (urlPattern !== "") {
        runContext.urlPattern = urlPattern;
      }
      return runContext;
    }
    get prehideSelectors() {
      return this.r[5].map((i) => this.s[i].toString());
    }
    get detectCmp() {
      return this.r[6].map(this._decodeRuleStep.bind(this));
    }
    get detectPopup() {
      return this.r[7].map(this._decodeRuleStep.bind(this));
    }
    get optOut() {
      return this.r[8].map(this._decodeRuleStep.bind(this));
    }
    get test() {
      return this.r[9].map(this._decodeRuleStep.bind(this));
    }
  };
  function clearUnusedStrings(ruleset, ignoreBeforeIndex = 0) {
    const { v, s, r } = ruleset;
    const usedStringIds = /* @__PURE__ */ new Set();
    function addStringIdsFromRuleSteps(steps) {
      steps.forEach((step) => {
        for (const [, shortKey] of compactedRuleSteps) {
          if (step[shortKey] !== void 0) {
            usedStringIds.add(step[shortKey]);
          }
        }
        if (step.if) {
          addStringIdsFromRuleSteps([step.if]);
        }
        if (step.then) {
          addStringIdsFromRuleSteps(step.then);
        }
        if (step.else) {
          addStringIdsFromRuleSteps(step.else);
        }
        if (step.any) {
          addStringIdsFromRuleSteps(step.any);
        }
      });
    }
    ruleset.r.forEach((rule) => {
      addStringIdsFromRuleSteps(rule[6]);
      addStringIdsFromRuleSteps(rule[7]);
      addStringIdsFromRuleSteps(rule[8]);
      addStringIdsFromRuleSteps(rule[9]);
      rule[5].forEach((id) => usedStringIds.add(id));
    });
    return {
      v,
      r,
      s: s.slice(0, Math.max(...usedStringIds) + 1).map((str, idx) => {
        if (idx < ignoreBeforeIndex || usedStringIds.has(idx)) {
          return str;
        }
        return "";
      })
    };
  }
  function shouldRunRuleInContext(rule, mainFrame, url) {
    const runContext = rule[4];
    if (mainFrame && runContext === 1) {
      return false;
    }
    if (!mainFrame && [20, 22, 10, 12].includes(runContext)) {
      return false;
    }
    const urlPattern = rule[3];
    if (urlPattern && urlPattern !== "" && url.match(urlPattern) === null) {
      return false;
    }
    return true;
  }
  function filterCompactRules(rules, context) {
    const { v, s, r, index } = rules;
    const { url, mainFrame } = context;
    const shouldRunInContext = (rule) => shouldRunRuleInContext(rule, mainFrame, url);
    if (!mainFrame) {
      const ruleset = {
        v,
        s: s.slice(0, index.frameStringEnd),
        r: r.slice(index.frameRuleRange[0], index.frameRuleRange[1]).filter(shouldRunInContext)
      };
      return clearUnusedStrings(ruleset);
    }
    const genericRules = r.slice(index.genericRuleRange[0], index.genericRuleRange[1]);
    const specificRules = r.slice(index.specificRuleRange[0], index.specificRuleRange[1]).filter(shouldRunInContext);
    if (specificRules.length > 0) {
      const ruleset = {
        v,
        s,
        r: [...genericRules, ...specificRules]
      };
      return clearUnusedStrings(ruleset);
    }
    return {
      v,
      s: s.slice(0, index.genericStringEnd),
      r: genericRules
    };
  }

  // lib/web.ts
  function filterCMPs(rules, config) {
    return rules.filter((cmp) => {
      return (!config.disabledCmps || !config.disabledCmps.includes(cmp.name)) && // CMP is not disabled
      (config.enableCosmeticRules || !cmp.isCosmetic) && // CMP is not cosmetic or cosmetic rules are enabled
      (config.enableGeneratedRules || !cmp.name.startsWith("auto_"));
    });
  }
  var _config;
  var AutoConsent = class {
    constructor(sendContentMessage, config = null, declarativeRules = null) {
      this.id = getRandomID();
      this.rules = [];
      __privateAdd(this, _config);
      this.state = {
        lifecycle: "loading",
        prehideOn: false,
        findCmpAttempts: 0,
        detectedCmps: [],
        detectedPopups: [],
        heuristicPatterns: [],
        heuristicSnippets: [],
        selfTest: null,
        clicks: 0,
        startTime: 0,
        endTime: 0
      };
      evalState.sendContentMessage = sendContentMessage;
      this.sendContentMessage = sendContentMessage;
      this.rules = [];
      this.updateState({ lifecycle: "loading" });
      this.addDynamicRules();
      if (config) {
        this.initialize(config, declarativeRules);
      } else {
        if (declarativeRules) {
          this.parseDeclarativeRules(declarativeRules);
        }
        const initMsg = {
          type: "init",
          url: window.location.href
        };
        sendContentMessage(initMsg);
        this.updateState({ lifecycle: "waitingForInitResponse" });
      }
      this.domActions = new DomActions(this);
    }
    get config() {
      if (!__privateGet(this, _config)) {
        throw new Error("AutoConsent is not initialized yet");
      }
      return __privateGet(this, _config);
    }
    initialize(config, declarativeRules) {
      const normalizedConfig = normalizeConfig(config);
      normalizedConfig.logs.lifecycle && console.log("autoconsent init", window.location.href);
      __privateSet(this, _config, normalizedConfig);
      if (!normalizedConfig.enabled) {
        normalizedConfig.logs.lifecycle && console.log("autoconsent is disabled");
        return;
      }
      if (declarativeRules) {
        this.parseDeclarativeRules(declarativeRules);
      }
      this.rules = filterCMPs(this.rules, normalizedConfig);
      if (this.shouldPrehide) {
        if (document.documentElement) {
          this.prehideElements();
        } else {
          const delayedPrehide = () => {
            window.removeEventListener("DOMContentLoaded", delayedPrehide);
            this.prehideElements();
          };
          window.addEventListener("DOMContentLoaded", delayedPrehide);
        }
      }
      if (document.readyState === "loading") {
        const onReady = () => {
          window.removeEventListener("DOMContentLoaded", onReady);
          this.start();
        };
        window.addEventListener("DOMContentLoaded", onReady);
      } else {
        this.start();
      }
      this.updateState({ lifecycle: "initialized" });
    }
    get shouldPrehide() {
      return this.config.enablePrehide && !this.config.visualTest;
    }
    saveFocus() {
      this.focusedElement = document.activeElement;
      if (this.focusedElement) {
        this.config.logs.lifecycle && console.log("saving focus", this.focusedElement, location.href);
      }
    }
    restoreFocus() {
      if (this.focusedElement) {
        this.config.logs.lifecycle && console.log("restoring focus", this.focusedElement, location.href);
        try {
          this.focusedElement.focus({ preventScroll: true });
        } catch (e) {
          this.config.logs.errors && console.warn("error restoring focus", e);
        }
        this.focusedElement = void 0;
      }
    }
    addDynamicRules() {
      dynamicCMPs.forEach((Cmp) => {
        this.rules.push(new Cmp(this));
      });
    }
    parseDeclarativeRules(declarativeRules) {
      const perfEnabled = __privateGet(this, _config)?.performanceLoggingEnabled;
      perfEnabled && performance.mark("parseDeclarativeRulesStart");
      if (declarativeRules.autoconsent) {
        declarativeRules.autoconsent.forEach((ruleset) => {
          this.addDeclarativeCMP(ruleset);
        });
      }
      if (declarativeRules.compact) {
        try {
          const rules = decodeRules(declarativeRules.compact);
          rules.forEach(this.addDeclarativeCMP.bind(this));
        } catch (e) {
          __privateGet(this, _config)?.logs.errors && console.error(e);
        }
      }
      perfEnabled && performance.mark("parseDeclarativeRulesEnd");
      perfEnabled && performance.measure("parseDeclarativeRules", "parseDeclarativeRulesStart", "parseDeclarativeRulesEnd");
    }
    addDeclarativeCMP(ruleset) {
      if ((ruleset.minimumRuleStepVersion || 1) <= SUPPORTED_RULE_STEP_VERSION) {
        this.rules.push(new AutoConsentCMP(ruleset, this));
      }
    }
    // start the detection process, possibly with a delay
    start() {
      scheduleWhenIdle(() => this._start());
    }
    async _start() {
      const logsConfig = this.config.logs;
      logsConfig.lifecycle && console.log(`Detecting CMPs on ${window.location.href}`);
      this.updateState({ lifecycle: "started" });
      const foundCmps = await this.findCmp(this.config.detectRetries);
      this.updateState({ detectedCmps: foundCmps.map((c) => c.name) });
      if (this.config.performanceLoggingEnabled) {
        this.updateState({ performance: this.measurePerformance() });
      }
      if (foundCmps.length === 0) {
        logsConfig.lifecycle && console.log("no CMP found", location.href);
        if (this.shouldPrehide) {
          this.undoPrehide();
        }
        this.updateState({ lifecycle: "nothingDetected" });
        return false;
      }
      this.updateState({ lifecycle: "cmpDetected" });
      const staticCmps = [];
      const cosmeticCmps = [];
      for (const cmp of foundCmps) {
        if (cmp.isCosmetic) {
          cosmeticCmps.push(cmp);
        } else {
          staticCmps.push(cmp);
        }
      }
      let result = false;
      let foundPopups = await this.detectPopups(staticCmps, async (cmp) => {
        result = await this.handlePopup(cmp);
      });
      if (foundPopups.length === 0) {
        foundPopups = await this.detectPopups(cosmeticCmps, async (cmp) => {
          result = await this.handlePopup(cmp);
        });
      }
      if (foundPopups.length === 0) {
        logsConfig.lifecycle && console.log("no popup found");
        if (this.shouldPrehide) {
          this.undoPrehide();
        }
        return false;
      }
      if (foundPopups.length > 1) {
        const errorDetails = {
          msg: `Found multiple CMPs, check the detection rules.`,
          cmps: foundPopups.map((cmp) => cmp.name)
        };
        logsConfig.errors && console.warn(errorDetails.msg, errorDetails.cmps);
        this.sendContentMessage({
          type: "autoconsentError",
          details: errorDetails
        });
      }
      return result;
    }
    async findCmp(retries) {
      const logsConfig = this.config.logs;
      this.updateState({ findCmpAttempts: this.state.findCmpAttempts + 1 });
      const foundCMPs = [];
      const isTop = isTopFrame();
      const siteSpecificRules = [];
      const genericRules = [];
      this.rules.forEach((cmp) => {
        if (cmp.checkFrameContext(isTop)) {
          const isSiteSpecific = !!cmp.runContext.urlPattern;
          if (cmp.hasMatchingUrlPattern()) {
            siteSpecificRules.push(cmp);
          } else if (!isSiteSpecific) {
            genericRules.push(cmp);
          }
        }
      });
      const heuristicRules = isTop && this.config.heuristicMode !== "off" && this.state.findCmpAttempts % 2 === 0 ? [new AutoConsentHeuristicCMP(this, this.config.heuristicMode)] : [];
      const rulesPriorityStages = [
        ["site-specific", siteSpecificRules],
        ["generic", genericRules],
        ["heuristic", heuristicRules]
      ];
      const runDetectCmp = async (cmp) => {
        try {
          const result = await cmp.detectCmp();
          if (result) {
            logsConfig.lifecycle && console.log(`Found CMP: ${cmp.name} ${window.location.href}`);
            this.sendContentMessage({
              type: "cmpDetected",
              url: location.href,
              cmp: cmp.name
            });
            foundCMPs.push(cmp);
          }
        } catch (e) {
          logsConfig.errors && console.warn(`error detecting ${cmp.name}`, e);
        }
      };
      const mutationObserver = this.domActions.waitForMutation("html");
      mutationObserver.catch(() => {
      });
      for (const [stageName, ruleGroup] of rulesPriorityStages) {
        logsConfig.lifecycle && ruleGroup.length > 0 && console.log(
          `Trying ${stageName} rules`,
          ruleGroup.map((r) => r.name)
        );
        this.config.performanceLoggingEnabled && performance.mark(`findCmpStage_${stageName}`);
        await Promise.all(ruleGroup.map(runDetectCmp));
        this.config.performanceLoggingEnabled && performance.mark(`findCmpStageEnd_${stageName}`);
        this.config.performanceLoggingEnabled && performance.measure(`findCmp_${stageName}`, `findCmpStage_${stageName}`, `findCmpStageEnd_${stageName}`);
        if (foundCMPs.length > 0) {
          break;
        }
      }
      this.detectHeuristics();
      if (foundCMPs.length === 0 && retries > 0) {
        const waitFor2 = [this.domActions.wait(500)];
        if (this.state.findCmpAttempts > 1) {
          waitFor2.push(mutationObserver);
        }
        try {
          await Promise.all(waitFor2);
        } catch (e) {
          return [];
        }
        return this.findCmp(retries - 1);
      }
      return foundCMPs;
    }
    detectHeuristics() {
      if (this.config.enableHeuristicDetection) {
        this.config.performanceLoggingEnabled && performance.mark("detectHeuristicsStart");
        const { patterns, snippets: snippets2 } = checkHeuristicPatterns(document.documentElement?.innerText || "");
        if (patterns.length > 0 && (patterns.length !== this.state.heuristicPatterns.length || this.state.heuristicPatterns.some((p, i) => p !== patterns[i]))) {
          this.config.logs.lifecycle && console.log("Heuristic patterns found", patterns, snippets2);
          this.updateState({ heuristicPatterns: patterns, heuristicSnippets: snippets2 });
        }
        this.config.performanceLoggingEnabled && performance.mark("detectHeuristicsEnd");
        this.config.performanceLoggingEnabled && performance.measure("detectHeuristics", "detectHeuristicsStart", "detectHeuristicsEnd");
      }
    }
    /**
     * Detect if a CMP has a popup open. Fullfils with the CMP if a popup is open, otherwise rejects.
     */
    async detectPopup(cmp) {
      const isOpen = await this.waitForPopup(cmp).catch((error) => {
        this.config.logs.errors && console.warn(`error waiting for a popup for ${cmp.name}`, error);
        return false;
      });
      if (isOpen) {
        this.updateState({ detectedPopups: this.state.detectedPopups.concat([cmp.name]) });
        this.sendContentMessage({
          type: "popupFound",
          cmp: cmp.name,
          url: location.href
        });
        return cmp;
      }
      throw new Error("Popup is not shown");
    }
    /**
     * Detect if any of the CMPs has a popup open. Returns a list of CMPs with open popups.
     */
    async detectPopups(cmps, onFirstPopupAppears) {
      const tasks = cmps.map((cmp) => this.detectPopup(cmp));
      await Promise.any(tasks).then((cmp) => {
        this.detectHeuristics();
        onFirstPopupAppears(cmp);
      }).catch(() => {
      });
      const results = await Promise.allSettled(tasks);
      const popups = [];
      for (const result of results) {
        if (result.status === "fulfilled") {
          popups.push(result.value);
        }
      }
      return popups;
    }
    async handlePopup(cmp) {
      this.updateState({ lifecycle: "openPopupDetected", startTime: Date.now() });
      if (this.shouldPrehide && !this.state.prehideOn) {
        this.prehideElements();
      }
      this.foundCmp = cmp;
      if (this.config.autoAction === "optOut") {
        return await this.doOptOut();
      } else if (this.config.autoAction === "optIn") {
        return await this.doOptIn();
      } else {
        this.config.logs.lifecycle && console.log("waiting for opt-out signal...", location.href);
        return true;
      }
    }
    async doOptOut() {
      const logsConfig = this.config.logs;
      this.updateState({ lifecycle: "runningOptOut" });
      this.saveFocus();
      let optOutResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to opt out");
        optOutResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: opt out on ${window.location.href}`);
        optOutResult = await this.foundCmp.optOut();
        logsConfig.lifecycle && console.log(`${this.foundCmp.name}: opt out result ${optOutResult}`);
      }
      if (this.shouldPrehide) {
        this.undoPrehide();
      }
      this.sendContentMessage({
        type: "optOutResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: optOutResult,
        scheduleSelfTest: Boolean(this.foundCmp && this.foundCmp.hasSelfTest),
        url: location.href
      });
      if (optOutResult && this.foundCmp && !this.foundCmp.isIntermediate) {
        this.state.endTime = Date.now();
        logsConfig.lifecycle && console.log(
          `${this.foundCmp.name}: done in ${this.state.endTime - this.state.startTime}ms with ${this.state.clicks} clicks`
        );
        this.sendContentMessage({
          type: "autoconsentDone",
          cmp: this.foundCmp?.name,
          isCosmetic: this.foundCmp?.isCosmetic,
          url: location.href,
          duration: this.state.endTime - this.state.startTime,
          totalClicks: this.state.clicks
        });
        this.updateState({ lifecycle: "done" });
      } else {
        this.updateState({ lifecycle: optOutResult ? "optOutSucceeded" : "optOutFailed" });
      }
      this.restoreFocus();
      return optOutResult;
    }
    async doOptIn() {
      const logsConfig = this.config.logs;
      this.updateState({ lifecycle: "runningOptIn" });
      this.saveFocus();
      let optInResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to opt in");
        optInResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: opt in on ${window.location.href}`);
        optInResult = await this.foundCmp.optIn();
        logsConfig.lifecycle && console.log(`${this.foundCmp.name}: opt in result ${optInResult}`);
      }
      if (this.shouldPrehide) {
        this.undoPrehide();
      }
      this.sendContentMessage({
        type: "optInResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: optInResult,
        scheduleSelfTest: false,
        // self-tests are only for opt-out at the moment
        url: location.href
      });
      if (optInResult && this.foundCmp && !this.foundCmp.isIntermediate) {
        this.state.endTime = Date.now();
        logsConfig.lifecycle && console.log(
          `${this.foundCmp.name}: done in ${this.state.endTime - this.state.startTime}ms with ${this.state.clicks} clicks`
        );
        this.sendContentMessage({
          type: "autoconsentDone",
          cmp: this.foundCmp.name,
          isCosmetic: this.foundCmp.isCosmetic,
          url: location.href,
          duration: this.state.endTime - this.state.startTime,
          totalClicks: this.state.clicks
        });
        this.updateState({ lifecycle: "done" });
      } else {
        this.updateState({ lifecycle: optInResult ? "optInSucceeded" : "optInFailed" });
      }
      this.restoreFocus();
      return optInResult;
    }
    async doSelfTest() {
      const logsConfig = this.config.logs;
      let selfTestResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to self test");
        selfTestResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: self-test on ${window.location.href}`);
        selfTestResult = await this.foundCmp.test();
      }
      this.sendContentMessage({
        type: "selfTestResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: selfTestResult,
        url: location.href
      });
      this.updateState({ selfTest: selfTestResult });
      return selfTestResult;
    }
    async waitForPopup(cmp, retries = 10, interval = 500) {
      const logsConfig = this.config.logs;
      logsConfig.lifecycle && console.log("checking if popup is open...", cmp.name);
      let mutationObserver = null;
      if (this.config.enablePopupMutationObserver) {
        mutationObserver = this.domActions.waitForMutation("html", 1e4);
        mutationObserver.catch(() => {
        });
      }
      const isOpen = await cmp.detectPopup().catch((e) => {
        logsConfig.errors && console.warn(`error detecting popup for ${cmp.name}`, e);
        return false;
      });
      if (!isOpen && retries > 0) {
        if (mutationObserver) {
          try {
            await Promise.all([this.domActions.wait(interval), mutationObserver]);
          } catch (e) {
            logsConfig.lifecycle && console.log(cmp.name, "popup detection timed out waiting for DOM mutation");
          }
        } else {
          await this.domActions.wait(interval);
        }
        return this.waitForPopup(cmp, retries - 1, interval);
      }
      logsConfig.lifecycle && console.log(cmp.name, `popup is ${isOpen ? "open" : "not open"}`);
      return isOpen;
    }
    prehideElements() {
      const logsConfig = this.config.logs;
      const globalHidden = [
        "#didomi-popup,.didomi-popup-container,.didomi-popup-notice,.didomi-consent-popup-preferences,#didomi-notice,.didomi-popup-backdrop,.didomi-screen-medium"
      ];
      const selectors = this.rules.filter((rule) => rule.prehideSelectors && rule.checkRunContext()).reduce((selectorList, rule) => [...selectorList || [], ...rule.prehideSelectors || []], globalHidden);
      this.updateState({ prehideOn: true });
      setTimeout(() => {
        if (this.shouldPrehide && this.state.prehideOn && !["runningOptOut", "runningOptIn"].includes(this.state.lifecycle)) {
          logsConfig.lifecycle && console.log("Process is taking too long, unhiding elements");
          this.undoPrehide();
        }
      }, this.config.prehideTimeout || 2e3);
      return this.domActions.prehide(selectors.join(","));
    }
    undoPrehide() {
      this.updateState({ prehideOn: false });
      this.domActions.undoPrehide();
    }
    updateState(change) {
      Object.assign(this.state, change);
      this.sendContentMessage({
        type: "report",
        instanceId: this.id,
        url: window.location.href,
        mainFrame: isTopFrame(),
        state: this.state
      });
    }
    async receiveMessageCallback(message) {
      const logsConfig = __privateGet(this, _config)?.logs;
      if (logsConfig?.messages) {
        console.log("received from background", message, window.location.href);
      }
      switch (message.type) {
        case "initResp":
          this.initialize(message.config, message.rules);
          break;
        case "optIn":
          await this.doOptIn();
          break;
        case "optOut":
          await this.doOptOut();
          break;
        case "selfTest":
          await this.doSelfTest();
          break;
        case "evalResp":
          resolveEval(message.id, message.result);
          break;
        case "measurePerformance":
          this.updateState({ performance: this.measurePerformance() });
          break;
      }
    }
    measurePerformance() {
      const getRoundedPerformanceEntries = (name) => performance.getEntriesByName(name).map((m) => Number(m.duration.toFixed(3)));
      return {
        detectHeuristics: getRoundedPerformanceEntries("detectHeuristics"),
        heuristicDetector: getRoundedPerformanceEntries("heuristicDetector"),
        findCmpSiteSpecific: getRoundedPerformanceEntries("findCmp_site-specific"),
        findCmpGeneric: getRoundedPerformanceEntries("findCmp_generic"),
        findCmpHeuristic: getRoundedPerformanceEntries("findCmp_heuristic"),
        parseDeclarativeRules: getRoundedPerformanceEntries("parseDeclarativeRules")
      };
    }
  };
  _config = new WeakMap();

  // rules/compact-rules.json
  var compact_rules_default = { v: 1, s: ["dialog.cookie-consent", "dialog.cookie-consent form.cookie-consent__form", "dialog.cookie-consent form.cookie-consent__form button[value=no]", "dialog.cookie-consent form.cookie-consent__form button.cookie-consent__options-toggle", 'dialog.cookie-consent form.cookie-consent__form button[value="save_options"]', "div.acris-cookie-consent", "[data-acris-cookie-consent]", ".acris-cookie-consent.is--modal", "#ccAcceptOnlyFunctional", "#cookie-banner", "#adopt-controller-button", "#adopt-reject-all-button", "#adroll_consent_container", "#adroll_consent_reject", "__adroll_fpc", ".c-cookie-notice button[data-qa='allow-all-cookies']", ".c-cookie-notice", 'button[data-qa="reject-non-essentials"]', "serif_manage_cookies_viewed", "serif_allow_analytics", '#sp-cc-wrapper,[data-action=sp-cc],span[data-action="sp-cc"][data-sp-cc*="rejectAllAction"]', '#sp-cc-wrapper *,[data-action=sp-cc],span[data-action="sp-cc"][data-sp-cc*="rejectAllAction"]', "#sp-cc-rejectall-link", "#user-consent-management-granular-banner-overlay", "[data-testid=granular-banner-button-decline-all]", "div:has(> p > a[href='https://www.anthropic.com/legal/cookies'])", "div:has(> p > a[href='https://www.anthropic.com/legal/cookies']) button:nth-child(2)", "anthropic-consent-preferences", "iframe[title='Consent window']", "script[src*='cdn.appconsent.io/tcf2-clear/'][src*='core.bundle.js']", "iframe[srcdoc*='frame-root']", "script[src*='cdn.appconsent.io/tcf2/'][src*='core.bundle.js']", "#consent-tracking", "#consent-tracking .decline.btn", ".modal-open bahf-cookie-disclaimer-dpl3", "bahf-cookie-disclaimer-dpl3", "bahf-cookie-disclaimer-dpl3:not([aria-hidden=true])", "cookie_consent=denied", 'dialog[data-testid="cookie-message-modal"]', '[data-testid="cookie-message-bottom-sheet"]', '[data-testid="cookie-message-bottom-sheet-overlay"]', 'dialog[data-testid="cookie-message-modal"] button[data-testid="decline-all-cookies"], [data-testid="cookie-message-bottom-sheet"] button[data-testid="decline-all-cookies"]', 'dialog[data-testid="cookie-message-modal"], [data-testid="cookie-message-bottom-sheet"]', "#cookie-policy-info,#cookie-policy-info-bg", "#cookie-policy-info button", "#cookie-policy-info .btn-reject", "#cookie-policy-info .btn-setting", "#cookie-policy-info .btn-ok", 'form[class*="cookie-banner"][method="post"]', 'form[class*="cookie-banner"] div[class*="simple-options"] a[class*="customize-button"]', "input[type=checkbox][checked]:not([disabled])", 'a[class*="accept-selection-button"]', "#awsccc-cb-content", "#awsccc-cs-container", "#awsccc-cs-modalOverlay", "#awsccc-cs-container-inner", "button[data-id=awsccc-cb-btn-decline]", "button[data-id=awsccc-cb-btn-customize]", "input[aria-checked]", "input[aria-checked=true]", "button[data-id=awsccc-cs-btn-save]", ".axeptio_widget,.axeptio_mount", ".axeptio-widget--open", ".axeptio_mount .axeptio_widget", ".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton", "axeptio_authorized_vendors=%2C%2C", "#cookie_consent_wrapper, #cookie_consent_min_wrapper, #manage_cookie_consent_modal", "#cookie_consent_wrapper:not(.hideConsent)", "#cookie_consent_wrapper #consent_accept_essential, #manage_cookie_consent_modal #modal_consent_accept_essential", "#cookie_consent_wrapper.non_cookie_consent_wrapper #js_cookieBannerCloseBtn", "#cookie_consent_wrapper #consent_accept_essential", "#cookie_consent_wrapper #js_cookieBannerCloseBtn", "#manage_cookie_consent_modal #modal_consent_accept_essential", "#cookie_consent_min_wrapper #close_minimized_banner", "cookieConsent=1", "cookieConsent=5", ".b-cookie.is-open", ".cookie-alert.t-dark", "#cookie-accept-necessary", ".cookie-alert .checkbox__input:checked:not(:disabled)", ".cookie-alert__button button", "iabbb-cookies-message > section", "iabbb-cookies-message", "iabbb-cookies-message > section [name='allow-all']", "iabbb-cookies-message > section button.bds-button-unstyled", "iabbb-cookies-message dialog[open]", "iabbb-cookies-message dialog fieldset input[type=radio][value=false]:not(:checked)", "iabbb-cookies-message dialog button.bds-button", "iabbb-cookies-message > section button.bds-button:not([name])", ".cookiesgdpr__base:has(.cookiesgdpr__rejectbtn)", ".cookiesgdpr__base .cookiesgdpr__rejectbtn", ".cookiesgdpr__base .cookiesgdpr__wrapper", '"analitica":false', "#consent-manager", "#consent-manager button", "#consent-manager button:nth-child(2)", "tracking-preferences", "#bnp_container", "#bnp_cookie_banner", "#bnp_btn_accept,#bnp_btn_reject", "#bnp_btn_reject", "AD=0", ".cookie-notification", "#blocksy-ext-cookies-consent-styles-css", ".cookie-notification .ct-cookies-decline-button", "blocksy_cookies_consent_accepted=no", "#BorlabsCookieBox", "._brlbs-block-content,.brlbs-cmpnt-dialog", "._brlbs-bar-wrap,._brlbs-box-wrap,.brlbs-cmpnt-dialog", ".brlbs-btn-accept-only-essential,a[data-cookie-refuse]", ".brlbs-btn-save,#CookieBoxSaveButton", '.brlbs-cmpnt-close-button[data-borlabs-cookie-actions="close-button"]', "a[data-cookie-individual]", ".cookie-preference", "input[data-borlabs-cookie-checkbox]:checked", "#CookiePrefSave", "#bsw-consentCookie", "#bsw-consentCookie .accept-btn", "#bsw-consentCookie .manage-cookies-btn", "#bsw-consentCookie .config-banner_section", "#bsw-consentCookie input[type=checkbox]:checked:not(:disabled)", "x-bsw-consentCookie", ".bpa-cookie-banner", ".bpa-cookie-banner .bpa-module-full-hero", ".bpa-close-button", "cookie-allow-tracking=0", ".cookie-modal", "#notice-cookie-block", "#html-body #notice-cookie-block", "#btn-cookie-manage", "#notice-cookie-block input:checked", "#btn-cookie-save", "#html-body #notice-cookie-block, #notice-cookie", ".cassie-cookie-module", ".cassie-pre-banner", "#cassie_pre_banner_text", ".cassie-reject-all", ".cc_banner-wrapper", ".cc_banner", ".cc-banner[data-cc-banner]", ".cc-banner[data-cc-banner] button[data-cc-action=reject]", ".cc-banner[data-cc-banner] button[data-cc-action=preferences]", ".cc-preferences[data-cc-preferences]", ".cc-preferences[data-cc-preferences] input[type=radio][data-cc-action=toggle-category][value=off]", ".cc-preferences[data-cc-preferences] button[data-cc-action=reject]", ".cc-preferences[data-cc-preferences] button[data-cc-action=save]", ".c24-cookie-consent-wrapper", ".c24-cookie-consent-wrapper .c24-cookie-consent-functional", ".c24-cookie-consent-notice", "[data-modal-content]:has([data-toggle-target^='cookie'])", "[data-toggle-target^='cookie']", "[data-cookie-dismiss-all]", "#cp-gdpr-choices", ".gdpr-top-content > button", ".gdpr-top-back", ".gdpr-btm__right > button:nth-child(1)", "#ccc-module,#ccc-overlay,#ccc", "#ccc-module,#ccc-notify", "#ccc", ".ccc-reject-button", "#ccc-module", "#ccc-dismiss-button", ".cc-individual-cookie-settings", ".cc-individual-cookie-settings #cookie-settings-reject-all", "ckies_cookielaw", "#cl-consent", '#cl-consent [data-role="b_options"]', '.cl-consent-popup.cl-consent-visible [data-role="alloff"]', '[data-role="b_save"]', "__lxG__consent__v2_daisybit=", ".consent-modal[role=dialog]", "#consent_reject", "#manage_cookie_preferences", "#cookie_consent_preferences input:checked", "#consent_save", "ctc_rejected=1", ".cf_modal_container", "#cmplz-cookiebanner-container", "#cmplz-cookiebanner-container .cmplz-cookiebanner", ".cmplz-cookiebanner .cmplz-deny", "cmplz_banner-status=dismissed", '.cc-type-categories[aria-describedby="cookieconsent:desc"]', '.cc-type-categories[aria-describedby="cookieconsent:desc"] .cc-dismiss', ".cc-dismiss", ".cc-type-categories input[type=checkbox]:not([disabled]):checked", ".cc-save", '.cc-type-info[aria-describedby="cookieconsent:desc"]', '.cc-type-info[aria-describedby="cookieconsent:desc"] .cc-compliance .cc-btn', ".cc-deny", '[aria-describedby="cookieconsent:desc"]', '[aria-describedby="cookieconsent:desc"].cc-type-opt-out', '[aria-describedby="cookieconsent:desc"].cc-type-opt-out:not(.cc-invisible)', ".cmp-pref-link", ".cmp-body [id*=rejectAll]", ".cmp-body .cmp-save-btn", '.cc-type-opt-in[aria-describedby="cookieconsent:desc"]', ".cc-settings", ".cc-settings-view", ".cc-settings-view input[type=checkbox]:not([disabled]):checked", ".cc-settings-view .cc-btn-accept-selected", "._flo-consent-root", "._flo-consent-root ._flo-consent-modal-contextual ._flo-consent-button--accept-selected", "._flo-consent-root ._flo-consent-modal:not(._flo-consent-modal-contextual)", "._flo-consent-modal:not(._flo-consent-modal-contextual) ._flo-consent-button--reject", "._flo-consent-button--settings", "._flo-consent-modal-contextual ._flo-consent-button--accept-selected", "._flo-consent-modal-contextual ._flo-consent-checkbox:checked:not(._flo-consent-checkbox--necessary)", "._flo-consent-root._flo-consent-hide", "#ncmp__tool .ncmp__banner", "#ncmp__tool .ncmp__banner.ncmp__active", "ncmp=", "csm-cookie-consent", "#cookie-information-template-wrapper", "#CcpaWrapper", "CookieInformationConsent=", "cookie-banner-element#ckb", ".spicy-consent-wrapper", ".spicy-consent-bar", ".js-decline-all-cookies", "#cookie-law-info-bar, #cookie-law-bg, .cli-popupbar-overlay", "#cookie-law-info-bar", "cookielawinfo-checkbox-non-necessary=yes", "#btn-cookie-settings", "#notice-cookie-block #allow-functional-cookies, #notice-cookie-block #btn-cookie-settings", "#allow-functional-cookies", ".modal-body", '.modal-body input:checked, .switch[data-switch="on"]', '[role="dialog"] .modal-footer button', "#cookiescript_injected", "#cookiescript_reject", "#cookiescript_manage", ".cookiescript_fsd_main", "#cookieAcceptBar.cookieAcceptBar", "#cbPopupWrap.cb-popup--open", "#cbPopupWrap #cbRefuseTrigger", "cbDataAccepted=1", "#cc--main", "html.show--consent #cm", "#s-all-bn", "#s-rall-bn", "#c-s-bn", "#s-sv-bn", "cc_cookie=", "#cc-main", "#cc-main .cm-wrapper", ".cm__btn[data-role=necessary]", ".cm__btn[data-role=show]", "#cc-main .pm__btn[data-role=necessary]", ".cc-cookies", ".cc-cookies .cc-cookie-accept", ".cc-cookies .cc-cookie-decline", "#cookiefirst-root,.cookiefirst-root,[aria-labelledby=cookie-preference-panel-title]", "#cookiefirst-root,.cookiefirst-root", "#cookiefirst-root button[data-cookiefirst-action],.cookiefirst-root button[data-cookiefirst-action],#btnAcceptAllCookies,#btnRejectAllCookies,#btnAdjustSettings,[aria-labelledby=cookie-preference-panel-title] button", "button[data-cookiefirst-action=reject]", "#btnRejectAllCookies", "button[data-cookiefirst-action=adjust]", "[data-cookiefirst-widget=modal]", "button[data-cookiefirst-action=save]", ".ch2-container", ".ch2-dialog", ".ch2-open-settings-btn, .ch2-open-personal-data-btn", ".ch2-settings", ".ch2-deny-all-btn", ".ch2-settings input[type=checkbox]:not([disabled]):checked", ".ch2-save-settings-btn", "cookiehub=", ".cookieinfo", ".cookieinfo-close", ".cookiejs-banner-wrapper", ".cookiejs-banner-wrapper:not(.modal)", ".cookiejs-banner-wrapper button.cookiejs-button", ".cookiejs-banner-wrapper #reject-cookies-btn", "analytics_cookies=false", ".cookiejs-banner-wrapper.modal", ".cookiejs-banner-wrapper .cookiejs-banner-text", ".cookiejs-banner-wrapper .cookiejs-banner-li:nth-of-type(2) button", "cookiejs_preferences", ".cky-overlay,.cky-consent-container", ".cky-consent-container", ".cky-consent-container [data-cky-tag=reject-button]", ".cky-consent-container [data-cky-tag=donotsell-button]", ".cky-modal [data-cky-tag=optout-option-toggle]", ".cky-modal [data-cky-tag=optout-option-toggle]:not(:checked)", ".cky-modal [data-cky-tag=optout-confirm-button]", ".cky-modal [data-cky-tag=optout-success]", ".cky-modal [data-cky-tag=optout-close]", ".cky-consent-container [data-cky-tag=settings-button]", ".cky-modal-open input[type=checkbox],.cky-consent-bar-expand input[type=checkbox]", ".cky-modal-open input[type=checkbox]:checked,.cky-consent-bar-expand input[type=checkbox]:checked", ".cky-modal [data-cky-tag=detail-save-button],.cky-consent-bar-expand [data-cky-tag=detail-save-button]", ".cky-consent-container,.cky-overlay,.cky-consent-bar-expand", "advertisement:no", "dialog:has(#cookie-banner-2025-form)", "#cookie-banner-2025-form", 'button[form="cookie-banner-2025-form"][name="accept_cookie"]', 'button[form="cookie-banner-2025-form"][name="accept_cookie"][value="selection"]', ".cookiealert", ".configurecookies", ".confirmcookies", "#ct-ultimate-gdpr-cookie-popup", "#ct_ultimate-gdpr-cookie-reject a,#ct-ultimate-gdpr-cookie-reject", "#ct-ultimate-gdpr-cookie-change-settings", '#ct-ultimate-gdpr-cookie-modal-slider-form input[type=radio][value="1"], #ct-ultimate-gdpr-cookie-modal-slider-form input[type=radio][value="2"]', "#ct-ultimate-gdpr-cookie-modal-body .save", "ct-ultimate-gdpr-cookie=", "#cookiebar", "#cookiebar .cookiebar-content", 'div[class*="CookiePopup__desktopContainer"]:has(div[class*="CookiePopup"])', 'div[class*="CookiePopup__desktopContainer"]', ".cookie-banner.show .cookie-banner__content-all-btn", ".cookie-banner__content-essential-btn", ".dg-consent-banner", "datagrail_consent", "#didomi-popup,.didomi-popup-container,.didomi-popup-notice,.didomi-consent-popup-preferences,#didomi-notice,.didomi-popup-backdrop,.didomi-screen-medium", "#didomi-host", '#didomi-popup, #didomi-notice, .didomi-popup-notice, .didomi-notice-banner, #didomi-host:not([aria-hidden="true"]):not(:empty)', "#didomi-notice-disagree-button,.didomi-continue-without-agreeing", '[data-project="mol-fe-cmp"]', '[data-project="mol-fe-cmp"] [class*=footer]', '[data-project="mol-fe-cmp"] button[class*=basic]', '[data-project="mol-fe-cmp"] div[class*="tabContent"]', '[data-project="mol-fe-cmp"] div[class*="toggle"][class*="enabled"]', "#mol-ads-cmp-iframe, div.mol-ads-cmp > form > div", "div.mol-ads-cmp > form > div", "div.mol-ads-ccpa--message > u > a", ".mol-ads-cmp--modal-dialog", "a.mol-ads-cmp-footer-privacy", "button.mol-ads-cmp--btn-secondary", '[data-testid="cookie-banner"][class*="cookie-banner-styles__StyledCookieBanner"]', '[data-testid="cookie-banner"] button[kind="BUTTON/FLAT_SECONDARY"]', "cookie_tracking_enabled", "#pg-root-shadow-host", ".cmp-popup_popup", ".cmp-app_gdpr", ".cmp-popup_popup .cmp-intro_rejectAll", ".cmp-popup_popup .cmp-details_rejectAll", "lpubconsent=", "#rgpd-popup", "#rgpd-custom-popup", "#rgpd-popup #rgpd-continue-without-accepting", "#rgpd-continue-without-accepting", "consentLevel=none", "#drupalorg-crosssite-gdpr", ".no", "div[data-testid=cookie-consent-modal-backdrop]", "div[data-testid=cookie-consent-message-contents]", "button[data-testid=cookie-consent-adjust-settings]", "button[data-testid=cookie-consent-preferences-save]", "cc_functional=0", "cc_targeting=0", "#cookieBox .block-cookie-consent", "#cookieBox #consentButton.block-cookie-consent--btn", "#cookieBox", "div[role='dialog'][aria-modal='true']:has([data-qa='accept-all-button'])", "[data-qa='accept-all-button']", "[data-qa='reject-non-essential-button']", "#ensNotifyBanner", ".ensModal", "#ensNotifyBanner[style*=block]", "#ensModalWrapper[style*=block]", "#ensNotifyBanner #ensRejectAll,#ensNotifyBanner #rejectAll,#ensNotifyBanner #ensRejectBanner,#ensNotifyBanner .rejectAll,#ensNotifyBanner #bannerRejectButton,#ensNotifyBanner #ensRejectAds", "#ensOpenModal,#modalOpenButton,#bannerOpenModal", ".ensCheckbox:checked:not(:disabled)", "#ensSave,#modalSaveButton,#bannerSave", "#ensCloseBanner", "epaas-consent-drawer-shell", "%22advertising%22%3A0", "#gdpr-single-choice-overlay", "#gdpr-privacy-settings", "button[data-gdpr-open-full-settings]", ".gdpr-overlay-body input", ".pea_cook_wrapper,.pea_cook_more_info_popover", ".pea_cook_wrapper", "euCookie", "body.eu-cookie-compliance-popup-open", ".eu-cookie-compliance-banner .decline-button, .eu-cookie-compliance-banner .accept-necessary, .eu-cookie-compliance-save-preferences-button", "cookie-agreed=2", "#ez-cookie-dialog-wrapper", "#ez-manage-settings", "#ez-cookie-dialog input[type=checkbox]", "#ez-cookie-dialog input[type=checkbox]:checked", "#ez-save-settings", "ez-consent-tcf", "#fast-cmp-root", "iframe#fast-cmp-iframe", "fastCMP-", "fedex-gdpr", "fedex-gdpr div", "fedex-gdpr .fxg-gdpr__reject-all-btn", "_svs=", 'astro-island[component-url*="CookiesBanner"] .cookies', 'astro-island[component-url*="CookiesBanner"] .cookies button.cookies__buttons--settings', 'astro-island[component-url*="CookiesBanner"] .cookies input[name="targeting"]:checked', 'astro-island[component-url*="CookiesBanner"] .cookies input[name="targeting"]', 'astro-island[component-url*="CookiesBanner"] .cookies button.cookies__buttons--accept', "cookiesPreferences=[%22necessary%22]", "#cookie-advice.fv-cookie-advice", "#cookie-advice .fv-cookie-advice__message a:not([href]), #cookie-advice .fv-cookie-advice__message button", ".fv-cookies-management", ':is(fv-cookies-management-sheet, .modal-content):has(.fv-cookies-management) button[class*="--primary"]', "fever_cookies_configuration", "#fides-overlay, #fides-overlay-wrapper", "#fides-overlay-wrapper #fides-banner", ".fides-banner-button", "#fides-banner .fides-reject-all-button", "#fides-banner .fides-manage-preferences-button", ".fides-reject-all-button", "[fs-consent-element=banner]", "[fs-consent-element=banner] [fs-consent-element=allow]", "[fs-consent-element=banner] [fs-consent-element=deny]", ".fc-consent-root,.fc-dialog-container,.fc-dialog-overlay,.fc-dialog-content", ".fc-consent-root", ".fc-dialog-container", ".fc-cta-do-not-consent,.fc-cta-manage-options", ".fc-preference-consent:checked,.fc-preference-legitimate-interest:checked", ".fc-confirm-choices", '[class*="gcb-modal-overlay"]', "[data-gcb-modal-show-prefs]", "[data-gcb-modal-save]", "gcbcl=a3|m3", ".overlay_bc_banner", 'a[href="https://gdpr-legal-cookie.myshopify.com/"]', ".overlay_bc_banner .banner-body", ".overlay_bc_banner *[data-cookie-save]", '[data-test-id="cookie-banner-body"]', 'cookie-banner [data-test-id="cookie-banner-body"] [data-test-id="reject-button"]', '[data-test-id="cookie-banner-body"] [data-test-id="reject-button"]', "#gtm_privacy", "pwinteraction=", 'a[href^="https://policies.google.com/technologies/cookies"', 'form[action^="https://consent.google."][action$="/save"],form[action^="https://consent.youtube."][action$="/save"]', 'form[action^="https://consent.google."][action$="/save"]:has(input[name=set_eom][value=true]) button,form[action^="https://consent.youtube."][action$="/save"]:has(input[name=set_eom][value=true]) button', ".glue-cookie-notification-bar", ".glue-cookie-notification-bar__reject", ".HTjtHe#xe7COe", '.HTjtHe#xe7COe a[href^="https://policies.google.com/technologies/cookies"]', ".HTjtHe#xe7COe button#W0wltc", "SOCS=CAE", ".govuk-cookie-banner__message", ".govuk-cookie-banner__message .govuk-button", ".gravitoCMP-background-overlay", "#modalSettingBtn.gravitoCMP-button", "#allRejectBtn", ".gravitoCMP-content-section", ".gravitoCMP-content input[type=checkbox]:checked", "gravitoData", "#modal-host > div.no-hash > div.window-wrapper", "#modal-host > div.no-hash > div.window-wrapper, div[data-testid=qualtrics-container]", '#modal-host > div.no-hash > div.window-wrapper > div:last-child a[href="/privacy-settings"]', "div#__next", "#__next div:nth-child(1) > button:first-child", ".cookie-modal .cookie-accept-btn", ".cookie-modal .js-cookie-reject-btn", "cookies_rejected=1", ".cookieModalContent", "#cookie-banner-overlay", "#manageCookie", ".cookieSettingsModal", "#AOCookieToggle", "#AOCookieToggle[aria-pressed=true]", "#TPCookieToggle", "#TPCookieToggle[aria-pressed=true]", "#updateCookieButton", "dialog[data-cookie-consent]:has([data-cookie-accept])", "dialog[data-cookie-consent] [data-cookie-accept]", 'a[data-cookie-accept="functional"]', '[data-test="cookie-consent-notification"]', '[data-test="cookie-consent-notification"] [data-test="accept-button"]', '[data-test="cookie-consent-notification"] [data-test="dismiss-button"]', '[data-test="cookie-consent-notification"] [data-test="settings-button"]', '[data-test="cookie-consent-modal"] [data-test="accept-button"]', "cookie_consent=", "#hu.hu-wrapper", "#hu.hu-visible", "#hu-cookies-save", "#hs-eu-cookie-confirmation", "#hs-eu-decline-button", "body > #idxrcookies", "body > #idxrcookies #idxrcookiesKO", "module-rgpd-component", "cookieConsent=consent_1", ".accept-cookie", ".accept-cookie > .accept-cookie-inner", "xpath///*[contains(@class, 'accept-cookie')]//*[contains(text(), 'Decline')]", "cookie-pref=rejected", ".privacy-consent--backdrop", ".privacy-consent--modal", ".footer-config-link", "#confirmSelection", "#iubenda-cs-banner", ".iubenda-cs-accept-btn", ".iubenda-cs-reject-btn", ".iubenda-cs-customize-btn", ".iub-btn-reject", "#iubFooterBtn", "body.cookies-request #cookie-bar", "body.cookies-request #cookie-bar .disallow-cookies", "cookie_permission_granted=no", ".widget_eu_cookie_law_widget", "div[class^=pecr-cookie-banner-]", "button[data-test^=manage-cookies]", "label[data-test^=toggle][class*=checked]:not([class*=disabled])", "button[data-test=save-preferences]", ".cookie-bar", ".cookie-bar .cookie-bar__message,.cookie-bar .cookie-bar__buttons", "cookies-state=accepted", ".consent-banner", ".consent-banner .consent-banner__actions", ".consent-banner__actions button.basic-button.secondary", ".consent-modal__footer button.basic-button.secondary", ".consent-modal .base-modal__body a", "label.consent-switch input[type=checkbox]:checked", ".consent-modal__footer button.basic-button.primary", ".kc-overlay", "#kconsent", ".kc-dialog", "#kc-denyAndHide", "#lanyard_root div[role='dialog']", "#ketch-banner", "#ketch-modal", "#lanyard_root [class*='ketch-fixed']", "#lanyard_root button[aria-label^='Reject' i], #ketch-banner button[aria-label^='Reject' i]", "#lanyard_root [aria-describedby=preference-description], #lanyard_root [aria-describedby=modal-description], #ketch-modal, #ketch-purposes-modal, #ketch-preferences", "#lanyard_root button[class*=confirmButton], #lanyard_root div[class*=actions_] > button:nth-child(1), #lanyard_root button[class*=actionButton], #ketch-modal button[aria-label='Confirm'], #ketch-purposes-modal button[aria-label='Confirm'], #ketch-modal button[aria-label='Save Settings'], #ketch-purposes-modal button[aria-label='Save Settings']", "#ketch-banner-button-tertiary", "#lanyard_root [aria-describedby=banner-description]", "#lanyard_root div[class*=buttons] > button[class*=secondaryButton], #lanyard_root button[class*=buttons-secondary], #ketch-banner-button-secondary", "#lanyard_root [id^='ketch-banner-buttons-container'] > button:only-child", "#lanyard_root [aria-describedby=preference-description],#lanyard_root [aria-describedby=modal-description], #ketch-preferences, #ketch-purposes-modal", "#lanyard_root button[class*=rejectButton], #lanyard_root button[class*=rejectAllButton], #ketch-modal button[aria-label='Reject All'], #ketch-purposes-modal button[aria-label='Reject All']", "#lanyard_root input[type=checkbox][role=switch][aria-checked=true]:not(:disabled)", "_ketch_consent_v1_", 'modal-box[data-options*="cookies"] .modal-object--cookie', ".modal-object--cookie [data-js-cookies-decline]", ".lia-cookie-banner-alert", ".lia-cookie-banner-alert .AjaxFeedback", ".lia-cookie-banner-alert a.lia-cookie-banner-alert-reject", ".darken-layer.open,.lightbox.lightbox--cookie-consent", "body.cookie-consent-is-active div.lightbox--cookie-consent > div.lightbox__content > div.cookie-consent[data-jsb]", ".cookie-consent__footer > button[type='submit']:not([data-button='selectAll'])", "#lgcookieslaw_banner,#lgcookieslaw_modal,#lgcookieslaw-modal,.lgcookieslaw-overlay,.lgcookieslaw_overlay", ".artdeco-global-alert[type=COOKIE_CONSENT]", ".artdeco-global-alert[type=COOKIE_CONSENT] button[action-type=DENY]", "#macaron_cookie_box", "#macaron_cookie_box .macaronbtn", ".macaronbtn.refuse", ".macaronbtn.letmechoose", ".macaronbtn.letmechoose.open", "#cookie_description .paragraph", "#cookie_description input[type=checkbox]:checked:not(:disabled)", ".macaronbtn.confirmselection", "_deCookiesConsent", ".mco-consent-pop-up", ".mco-consent-slide-up", ".mco-consent-slide-up__backdrop", ".mco-overlay:has(.mco-consent)", "#mco-consent", '[class*="mco-consent-"][class*="__button-decline"]', '"acceptedAll":false', "div[aria-labelledby=pwa-consent-layer-title]", "div[class^=StyledConsentLayerWrapper-]", "div[aria-labelledby^=pwa-consent-layer-title]", "button[data-test^=pwa-consent-layer-deny-all]", "#mensajeCookies", "#divConfigCookies", "#mensajeCookies .cajaCookies #btnAcceptAll", "#mensajeCookies #btnAcceptNoAllCookies", "#divConfigCookies #btnAcceptNoCookies", "#mensajeCookies .cajaCookies", "#mensajeCookies #btnConfigCookies", "#wcpConsentBannerCtrl", "#gdprCookieBar", "#gdprCookieBarOverlay", ".mst-gdpr__cookie-bar-wrapper", ".mst-gdpr__cookie-bar-overlay", ".mst-gdpr__cookie-bar .mst-gdpr__buttons", ".mst-gdpr__buttons [data-trigger-settings=decline],.mst-gdpr__buttons button[\\@click=handleDecline]", ".mst-gdpr__buttons [data-trigger-settings=decline]", ".mst-gdpr__buttons button[\\@click=handleDecline]", ".mst-gdpr__buttons [data-trigger-settings=trigger]", ".mst-gdpr__buttons button[\\@click=openSettings]", "[data-mst-gdpr-settings-apply],.mst-gdpr__cookie-settings--success-btn,[x-on\\:click=save]", "input.checkbox[data-group-id]:not([disabled]):checked", "[data-mst-gdpr-settings-apply]", ".mst-gdpr__cookie-settings--success-btn", "[x-on\\:click=save]", "gdpr_cookie_consent=", "#__consent .md-consent__inner", "#__consent .md-consent__overlay", "#__consent form[name='consent'] .md-consent__settings input[type='checkbox']", "#__consent:not([hidden]) .md-consent__inner", "#__consent .md-consent__settings input[type='checkbox']:checked", "#__consent .md-consent__controls button.md-button--primary", "dialog[data-testid=accept-our-cookies-dialog]", "#banner-manage", "#pc-confirm", "#moove_gdpr_cookie_info_bar", "#moove_gdpr_cookie_info_bar:not(.moove-gdpr-info-bar-hidden)", ".moove-gdpr-infobar-reject-btn", "#moove_gdpr_cookie_info_bar .change-settings-button", "#moove_gdpr_cookie_modal", ".moove-gdpr-modal-save-settings", "#nhsuk-cookie-banner", "#nhsuk-cookie-banner__link_accept", ".disc-cp--active", ".disc-cp-modal__modal", ".js-disc-cp-deny-all", ".tx-om-cookie-consent", ".tx-om-cookie-consent .active[data-omcookie-panel]", "[data-omcookie-panel-save=min]", "input[data-omcookie-panel-grp]:checked:not(:disabled)", "[data-omcookie-panel-save=save]", ".legalmonster-cleanslate", ".legalmonster-cleanslate #lm-cookie-wall-container", "#lm-accept-necessary", ".osano-cm-window,.osano-cm-dialog", ".osano-cm-window", ".osano-cm-dialog:not(.osano-cm-dialog--hidden)", ".osano-cm-denyAll,.osano-cm-deny", ".osano-cm-dialog--type_bar .osano-cm-dialog__close", ".cookieBanner--visibility", ".cookieBanner__wrapper", ".js_cookieBannerProhibitionButton", ".cookie-banner:has([data-ol-cookie-banner-set-consent])", "[data-ol-cookie-banner-set-consent]", "[data-ol-cookie-banner-set-consent=essential]", "oa", ".js-cookie-notice:has(#cookie_settings-form)", ".js-cookie-notice #cookie_settings-form", ".js-cookie-notice button[value=disable]", "#pandectes-banner", "#pandectes-banner .cc-deny", "#pandectes-banner .cc-settings", ".pd-cp-ui-rejectAll", ".pd-cp-ui-save", "#ccpaCookieContent_wrapper", "#ccpaCookieBanner", "#bannerDeclineButton", "a#manageCookiesLink", "type%3Dexplicit", "#gdprCookieBanner", "#gdprCookieContent_wrapper", "cookie_prefs", ".cookie-wrapper .cookie-tip", ".cookie-wrapper .cookie-tip__container", ".cookie-wrapper .cookie-tip__text", ".cookie-wrapper .cookie-tip__button > .button", ".el-dialog.cookie-dialog", ".el-dialog.cookie-dialog .el-switch.is-checked", ".el-dialog.cookie-dialog .el-dialog__footer button.el-button--primary", "allow_analysis=%22false%22", "allow_analysis=false", "#pmc-pp-tou--notice", "#js-consent-banner", "#js-consent-banner #js-cookie-reject-button", "#js-consent-banner #js-cookie-dismiss-button", "cookie-consent=", ".cookiesBanner", ".cookiesBanner .wrapper-cookies", ".gdpr-wrapper", ".gdpr-wrapper .gdpr-cookie-wrapper", "#cookie-bar", "#cookie-bar .cb-enable,#cookie-bar .cb-disable,#cookie-bar .cb-policy", "#cookie-bar .cb-disable", "cb-enabled=accepted", "#cookie-consent-banner", "#cookie-consent-banner #accept-button", "#cookie-consent-banner #deny-button", "#cookie-consent-banner #manage-settings-button", "#manage-cookies #save-button", "#pubtech-cmp", "#pubtech-cmp #pt-actions", "#pubtech-cmp #pt-close", "#qc-cmp2-main,#qc-cmp2-container", "#qc-cmp2-container", "#qc-cmp2-ui", "#disagree-btn", '.qc-cmp2-summary-buttons > button[mode="secondary"]', '.qc-cmp2-summary-buttons > button[mode="secondary"]:nth-of-type(2)', '.qc-cmp2-summary-buttons > button[mode="secondary"]:nth-of-type(1)', "#qc-cmp2-ui .qc-cmp2-consent-info", '.qc-cmp2-toggle-switch > button[aria-checked="true"]', ".qc-cmp2-buttons-desktop > button[mode=primary]", "#r42CookieBar", "#r42CookieBar .cw-deny", "#r42CookieBar[open] .cw-deny", "#concentBanner", "#concentPopup", "#concentBanner .rdc-save-preferences, #concentPopup .rdc-save-preferences", "#concentBanner, #concentPopup", 'div[consent-skip-blocker="1"][id][data-bg]', 'div[consent-skip-blocker="1"][id][data-bg] > dialog > div > div > div > div > div > a[role=button][id]', ".consent-banner .consent-banner-copy", ".consent-banner .consent--manage", ".consent-modal .consent-bucket", ".consent-modal input[type=checkbox]:checked", ".consent-modal .consent-save", ".cookies-consent", ".cookies-consent .cookies-inner", "s4s-privacy-module", "s4s-privacy-module button.decline", "_cookieanalytics", ".stpd_cmp_wrapper", ".stpd_cmp_wrapper .stpd_cmp_form", '.stpd_cmp_wrapper .stpd_flexed_btns button[type="submit"]', '.stpd_cmp_wrapper .stpd_flexed_btns button[type="button"]:not(.withdraw_consent)', ".stpd_cmp_wrapper .stpd_purposes_list", '.stpd_cmp_wrapper input[type="checkbox"]:checked', "euconsent-v2=", "#shopify-pc__banner", "#shopify-pc__banner__btn-decline", "sibbo-cmp-layout", "#rejectAllMain", "#sd-cmp", "#sd-cmp [role='button'][title='Close']", ".snigel-cmp-framework", "#sn-b-custom", "#sn-b-save", "snconsent", "#cookie_modal", "#cookie_modal #id_cookies_necessary", "#cookie_modal #id_cookies_all", ".cookie-banner-mount-point", ".cookie-banner-mount-point section[aria-label='Cookie banner']", ".cookie-banner-mount-point button.decline", "#cookie-consent-banner #accept-cdp-cookie", "#reject-cdp-cookie", "squiz.cdp.consent", ".cookiepreferences_popup", "#rejectAllButton", "#controlled_banner-cookie_consent-consent_banner", "#controlled_banner-cookie_consent-privacy_setting_banner", "#controlled_banner-cookie_consent-consent_banner [data-search-key-button=reject_all_text]", "#controlled_submit-cookie_consent-consent_banner-privacy_setting_banner_link", "#controlled_banner-cookie_consent-privacy_setting_banner [data-search-key-button=reject_all_text]", ".cookies-reminder", ".cookies-reminder .cookies-reminder__content", ".cookies-reminder .cookies-reminder__manage-button", "dialog[open] .cookies-select-modal .cookies-select-modal__buttons .ds-btn-apply-2-ds", " c=%7B%22essential", "div[class*=cookieBanner].pencraft", "hideCookieBanner=", "#CookieConsentBannerPlaceholder", "#CookieConsentBannerPlaceholder .cookies-banner-container", ".syno_cookie_element", ".syno_cookie_element .btn_option", ".syno_cookie_element .scb_dialog_body", ".syno_cookie_element.scb_dialog input[type=checkbox]:checked:not(:disabled):not([readonly])", ".syno_cookie_element.scb_dialog .scb_btn_save", "syno_confirm_v5_answer", '"targeting":false', "#consent-banner-main", "#consent-banner-modal", '#consent-banner-modal a[href="#reject"]', '#consent-banner-modal a[href="#settings"]', '#consent-banner-settings a[href="#reject"]', 'div[class^="cookies-banner-module_"]', 'div[class^="cookies-banner-module_cookie-banner_"]', 'div[class^="cookies-banner-module_small-cookie-banner_"]', "#tarteaucitronRoot", "#tarteaucitronAllDenied2", "#tarteaucitronCloseAlert", ".dsgvoaio-checkbox", ".dsgvoaio-checkbox > input:checked", "#tarteaucitronPersonalize", "#tarteaucitronAllDenied", "#tarteaucitronClosePanel", "#taunton-user-consent__overlay", "#taunton-user-consent__overlay:not([aria-hidden=true])", "#taunton-user-consent__toolbar input[type=checkbox]:checked", "#taunton-user-consent__toolbar button[type=submit]", "taunton_user_consent_submitted=true", "#tccCmpAlert", "xpath///span[contains(.,'Accetta solo cookie necessari')]", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs,#__tealiumImplicitmodal,#consent-layer", "#__tealiumGDPRecModal *,#__tealiumGDPRcpPrefs *,#__tealiumImplicitmodal *", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs,#__tealiumImplicitmodal", "#cm-acceptNone,.js-accept-essential-cookies,#continueWithoutAccepting,#no_consent,#consent_prompt_decline", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs", "#termly-code-snippet-support", "#termly-code-snippet-support div", '[data-tid="banner-decline"]', '.t-preference-button,[data-testid="preferences-link"]', ".t-declineAllButton", ".t-preference-modal input[type=checkbox][checked]:not([disabled])", ".t-saveButton", ".termsfeed-com---nb", ".cc-nb-reject", ".cc-nb-changep", 'input[cookie_consent_toggler="true"]', 'input[cookie_consent_toggler="true"]:checked', ".cc-cp-foot-save", ".cc_dialog.cc_css_reboot,.cc_overlay_lock", ".cc_dialog.cc_css_reboot", ".cc_dialog.cc_css_reboot .cc_b_cp", ".cookie-consent-preferences-dialog .cc_cp_f_save button", "#reject-all", "#privacy-test-page-cmp-test", "#privacy-test-page-cmp-test-prehide", "#privacy-test-page-cmp-test-banner", ".consent-banner-box", "consent-banner[component=consent-banner]", "button[data-consent=disagree]", "#cmpBanner", "#cookie-banner.visible", "#cookie-banner.visible .cookie-banner__container .cookie-banner__accept", "#cookie-banner.visible .cookie-banner__container .cookie-banner__reject", "TOYOTANATIONAL_ENSIGHTEN_PRIVACY_TargetingCookies", ".tp-dialog.cookie-dialog", ".tp-dialog.cookie-dialog button", ".tp-dialog-box .checkbox.clickable:not(.checked)", ".tp-dialog-box .tp-cookie-save", "tp_privacy_base", 'div.aem-page > div[class^="CookiesAlert_cookiesAlert__"]', "#transcend-consent-manager", "#shopify-section-cookies-controller", "#shopify-section-cookies-controller #cookies-controller-main-pane", "#cookies-controller-main-pane a[data-tab-target=manage-cookies]", "#manage-cookies-pane.active", "#manage-cookies-pane.active input[type=checkbox][checked]:not([disabled])", "#manage-cookies-pane.active button[type=submit]", ".truste_popframe.trustarc_newcm_container", "cmapi_cookie_privacy=permit 1", "cmapi_cookie_privacy=permit 1,2", "#truyo-consent-module", "#truyo-cookieBarContent", "button#declineAllCookieButton", ".react-cookie-policy", ".react-cookie-policy #cookie-policy", "xpath///div[@class='react-cookie-policy']//button[contains(., 'Cookie settings')]", "button[role='switch']", "xpath///li[contains(., 'Tracking')]//button[@role='switch'][@aria-checked='true']", "xpath///li[contains(., 'Advertising')]//button[@role='switch'][@aria-checked='true']", "xpath///button[contains(., 'Save and close')]", "trybe_cookies_consent_tracking=false", "iframe.tmblr-iframe--gdpr-banner", "#twcc__mechanism", "#twcc__mechanism .twcc__notice", "#twcc__decline-button", "twCookieConsent=", ".u12-data-protection-notice", ".u12-data-protection-notice button", ".u12-data-protection-notice button.js-decline", "dialog.cookie-policy", "dialog.cookie-policy header", 'xpath///*[@id="modal"]/div/header', "dialog header", "button.js-manage", 'xpath///*[@id="cookie-policy-content"]/p[4]/button[2]', "dialog.cookie-policy .p-switch__input:checked", "dialog.cookie-policy .js-save-preferences", 'xpath///*[@id="modal"]/div/button', "_cookies_accepted=essential", "#catapult-cookie-bar", ".has-cookie-bar #catapult-cookie-bar", "catAccCookies", "#usercentrics-root,#usercentrics-cmp-ui", "#usercentrics-cmp-ui", "#usercentrics-root", "#usercentrics-button", "#usercentrics-button #uc-btn-accept-banner", "#usercentrics-button #uc-btn-deny-banner", '[data-slot="dialog-content"] a[href*="vivenu.com/dataprivacy"]', '[data-slot="dialog-content"]:has(a[href*="vivenu.com/dataprivacy"]) button:nth-of-type(2)', "div[aria-labelledby=CookieAlertModalHeading]", "section[data-test=initial-waitrose-cookie-consent-banner]", "section[data-test=cookie-consent-modal]", "button[data-test=manage-cookies]", "button[data-test=submit]", "wtr_cookies_advertising=0", "wtr_cookies_analytics=0", ".fs-cc-components,[fs-cc=banner]", ".fs-cc-components,[fs-cc=banner] [fs-cc=allow]", "[fs-cc=banner] [fs-cc=deny]", "[fs-cc=banner]", "fs-cc", ".mw-cookiewarning-container", "[data-comp-type=cookie-banner-root-wix],[data-hook=ccsu-banner-wrapper]", "[data-hook=ccsu-banner-decline-all]", "[data-hook=ccsu-banner-wrapper]", ".wccom-comp-privacy-banner .wccom-privacy-banner", ".wccom-privacy-banner__content-buttons button.is-secondary", "div.wccom-modal__footer > button", '[data-automation-id="legalNotice"]', '[data-automation-id="legalNoticeDeclineButton"]', "enablePrivacyTracking=false", "#gdpr-cookie-consent-bar", "#gdpr-cookie-consent-bar #cookie_action_reject", "wpl_viewed_cookie=no", ".sp-dsgvo", ".sp-dsgvo.sp-dsgvo-popup-overlay", ".sp-dsgvo-privacy-btn-accept-nothing", "sp_dsgvo_cookie_settings", ".wpcc-container", ".wpcc-container .wpcc-message", "#wpconsent-root", "#wpconsent-container", "wpconsent_preferences=", ".cookie_warn", ".cookie_warn .cookie_warn_inner", "div[class^=cookie-consent-CookieConsent]", "#consent-settings-button", ".consent-banner-button-accept-overlay", "userConsent=%7B%22marketing%22%3Afalse", "#cookies-use-alert", ".disclaimer-opened #disclaimer-reject_cookies", ".disclaimer-opened #disclaimer-cookies", "#disclaimer-reject_cookies", "tp-yt-iron-overlay-backdrop.opened", "ytd-consent-bump-v2-lightbox", "ytd-consent-bump-v2-lightbox tp-yt-paper-dialog", 'ytd-consent-bump-v2-lightbox tp-yt-paper-dialog a[href^="https://consent.youtube.com/"]', "ytd-consent-bump-v2-lightbox .eom-buttons .eom-button-row:first-child ytd-button-renderer:first-child #button,ytd-consent-bump-v2-lightbox .eom-buttons .eom-button-row:first-child ytd-button-renderer:first-child button", ".consent-bump-v2-lightbox", "ytm-consent-bump-v2-renderer", "ytm-consent-bump-v2-renderer .privacy-terms + .one-col-dialog-buttons c3-material-button:nth-child(2) button, ytm-consent-bump-v2-renderer .privacy-terms + .one-col-dialog-buttons ytm-button-renderer:nth-child(2) button", "#zdf-cmp-banner-sdk", "#zdf-cmp-main.zdf-cmp-show", "#zdf-cmp-main #zdf-cmp-deny-btn", ".cookie-alert-extended", ".cookie-alert-extended-modal", "a[data-controller='cookie-alert/extended/detail-link']", ".cookie-alert-configuration-input:checked", "button[data-controller='cookie-alert/extended/button/configuration']", "#piano-cookie_banner", "#piano-cookie_banner [data-close-button]", 'body > div#root > div#ccpa-iframe-theme-provider[data-testid="ccpa-iframe-theme-provider"] > div#ccpa-iframe[data-testid="ccpa-iframe"] > div#ccpa_consent_banner[data-testid="ccpa_consent_banner"] > div:not([id]) > div:nth-child(3):not([id]) > span:not([id]) > span:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button#decline_cookies_button[data-testid="decline_cookies_button"]', "body:not([id]) > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#iubenda-cs-banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#ds-cookie-consent-banner > div:not([id]) > div:not([id]) > button:nth-child(4)#reject_optional_cookies", "body:not([id]) > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#handle-reject-all", "body > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1)#dashboard > div:not([id]) > div:nth-child(2)#dashboard-body-container > div:nth-child(2):not([id]) > button:nth-child(4)#decline-text", "body > div:not([id]) > div:nth-child(4):not([id]) > button#cookie_all_reject", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie_all_reject", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#handle-reject-all", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1)#dashboard > div:not([id]) > div:nth-child(1):not([id]) > button#decline-text", "body:not([id]) > div#root > div#ccpa-iframe-theme-provider > div#ccpa-iframe > div#ccpa_consent_banner > div:not([id]) > div:nth-child(3):not([id]) > span:not([id]) > span:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button#decline_cookies_button", "body#moneyinvesting > div#top > div#content > div:nth-child(4):not([id]) > a:nth-child(11):not([id])", "body > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div#app > div#app > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1):not([id]) > div:not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(4):not([id]) > div:not([id]) > a:nth-child(1)#ipoclick-btn-cookiebar-ik_ga_niet_akkoord_var_a", "body > div[id] > div > div > div > div > div:nth-child(3) > span > span > div:nth-child(2) > span:nth-child(1) > button", "div[class^=CookieBannerContent__Container]", "button[class^=CookieBannerContent__Settings]", "div[class^=CookiePreferencesModal__CategoryContainer] input:checked", "div[class^=CookiePreferencesModal__ButtonContainer] > button", "#overlay.cmp", "#overlay[role=dialog]", "#deny", "#edit-purpose-settings", "#save-purpose-settings", "ui_cid=OPTOUT", "#gdpr-consent-tool-wrapper", 'iframe[src^="https://cmp-consent-tool.privacymanager.io"]', "button#save", "#denyAll", ".okButton", "#manageSettings", ".purposes-overview-list", "button#saveAndExit", "span[role=checkbox][aria-checked=true]", "#cmp-app-container", "#save-all-pur", "#reminder", "adc-cookie-banner", "cookie_banner=closed", "[data-component=CookieBanner]", "[data-component=CookieBanner] [data-component=CookieBanner_AcceptAll]", "[data-component=CookieBanner] [data-component=CookieBanner_AcceptABCRequired]", "trackingconsent", '[data-testid="cookies_reject"]', "aside#cookies,.overlay-cookies", "#cookies .cookies-btn", "#cookies #submitCookies", "#cookies #rejectCookies", ".fullpageCover", ".fullpageCover a[href*='/go/page/privacy.html']", ".fullpageCover div.rounded-full:nth-child(2)", "#cookie-overlay", "#decline-cookies", "#aag-cookie-consent", "#gdpr-new-container", "#gdpr-new-container,#voyager-gdpr > div", "#voyager-gdpr > div", "#voyager-gdpr > div > div > button:nth-child(2)", "#gdpr-new-container button:nth-of-type(3)", "#gdpr-new-container button:nth-of-type(2)", "#gdpr-new-container button:first-of-type", "#voyager-gdpr-2025", "#voyager-gdpr-2025 button:last-of-type", "div:has(> div > button[allytmln=ally-cookie-consent])", "button[allytmln=ally-cookie-consent]", ".cookie-menu.js-cookie-banner", ".cookie-menu.js-cookie-banner .js-cookie-banner-close", "apaycnst-mktg=true", "#footer-container ~ div", "#footer-container > div", "body > div#shopify-section-popups > div:nth-child(1):not([id]) > modal-box#modal-popups-0 > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookieNotice > span:not([id])", "body > div#cookieAreaBase > div#cookieArea > p:nth-child(1):not([id]) > a:nth-child(3):not([id])", "body > div#cookieAreaBase > div#cookieArea > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body#customcss > div#root > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > span:nth-child(1):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", "body > div#cookieConsentModal > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(2):not([id])", 'body > div#cookie-banner-deezer > div#gdpr-dir-tag > div:not([id])[data-testid="cookie-banner"] > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#gdpr-btn-refuse-all[data-testid="gdpr-btn-refuse-all"]', "body:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div#gdpr_notification_container > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(6):not([id]) > div:not([id]) > a#gdpr_reject_button", "#cookiesPortletDiv", "#cookiesPortletDiv .cookie-buttons", "#cookiesPortletDiv button[aria-label='Only Mandatory']", "body > dialog:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > button:nth-child(2)#deny-consent-button", "body > div#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", ".MuiStack-root:has(a[href='/privacy']):has(button)", "body > div#portal-cookie-banner > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2)#portal-cookie-banner__wrapper > aside#portal-cookie-banner__content > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#messages > div:nth-child(1)#shopify-section-cookie-banner > section#shopify-section-cookie-banner > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#messages > div:nth-child(2)#shopify-section-newsletter-banner > section:nth-child(1):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > form#trackingConsent > button:nth-child(2):not([id])", "body:not([id]) > div#cookieConsent > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > pnp-root:not([id]) > div:not([id]) > ui-storefront:not([id]) > footer:nth-child(8):not([id]) > cx-page-layout:not([id]) > cx-page-slot:not([id]) > cms-popia-component:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#___gatsby > div:nth-child(1)#gatsby-focus-wrapper > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#layout-fea-0-bd-5-e-e-723-4027-aa-78-dbcc-846073-a-6 > div#page-62928 > div:not([id]) > div:nth-child(8)#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307 > div#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307-banner > div:nth-child(3):not([id]) > a:nth-child(1)#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307-decline", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div#c-cookiebanner > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > p:nth-child(1):not([id]) > button:not([id])", "body > div#wrapper > div:nth-child(4):not([id]) > button:nth-child(3):not([id])", "body > div#frame-modals > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#message-banner > div:not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#cookiemodal > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(5)#footer-cookie-buttons > a:nth-child(2)#footer-cookie-close", "body > div#notice-cookie-block > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#termsfeed-pc1-notice-banner > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(2)#termsfeed_privacy_consent_banner_button_reject_all", "body > main:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > hathi-cookie-consent-banner:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookies-modal > div:nth-child(2)#consent-modal > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id]) > span#consent-modal-refuse", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#body-wrapper > section:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#wrapwrap > div:nth-child(5)#website_cookies_bar > div:not([id]) > div:not([id]) > div:not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(1)#cookie-banner-essential", "body > aside:not([id]) > div:nth-child(2)#cookie-consent-modal > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#cc--main > div#cc_div > div:nth-child(1)#cm > div#c-inr > div:nth-child(2)#c-bns > button:nth-child(2)#c-s-bn", "body > div#dk-cookie-message > div:not([id]) > button:nth-child(3):not([id])", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > div#ch2-settings-dialog > div:nth-child(3):not([id]) > div:nth-child(1)#ch2-settings > button:nth-child(4):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:nth-child(3):not([id])", "body:not([id]) > div#cookieConsentBanner > div#cookieConsentContent > button:nth-child(4)#closeConsentBanner", "body:not([id]) > div#cookies-modal > div:nth-child(2)#consent-modal > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id]) > span#consent-modal-refuse", "body > div:not([id]) > a:nth-child(3):not([id])", "body > div#root > div:nth-child(2):not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#tna-cookie-prompt-banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#reject-cookie", 'body > div#ecom2-spa-root > div:nth-child(4)#layout-grid > div:nth-child(4)#main-content-v2 > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])[data-testid="cookie-btn-deny"]', "body > div:not([id]) > div:not([id]) > button:nth-child(5):not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > div#gdprCookieBar > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#c-footerBrandV1 > footer:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#ez-cookie-notification > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#ez-cookie-notification__decline", "body > div#site-wrapper > div:nth-child(1)#site-canvas > div#wrapper1 > main:nth-child(2)#mainSection1 > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#cookieadmin_reject_button", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:not([id])", "body > div#privacy-cookie-banners-root > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", 'body > div:not([id])[data-testid="CookieBanner"] > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > form:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", 'body > div#react-application > div:not([id]) > div:not([id])[data-testid="linaria-injector"] > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id])[data-testid="main-cookies-banner-container"] > section:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:not([id])', "body > div:not([id]) > div:nth-child(2):not([id]) > div#oax-dialog-main > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > aside#cmp-banner > form:not([id]) > div:nth-child(4):not([id]) > section:not([id]) > div:not([id]) > button:nth-child(1)#cmp-deny-all", "body > main#main > div:nth-child(6):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > a:not([id])", 'body > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])[data-testid="s-r-bu"]', "body > div#consent_manager-background > div:nth-child(1)#consent_manager-wrapper > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#consent_manager-accept-none", "body > div#ampsandConsentElement > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > section#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#cookie-banner > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-banner-reject", 'body > div:not([id])[data-popup=""][data-popup-cookies=""][data-terms-cookies-popup-common=""][data-popup-need-overlay="true"] > div:not([id])[data-terms-cookies-popup=""][data-terms-cookies-popup-common=""] > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])[data-cookies="disallow_all_cookies"]', "body > privacy-banner:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#hw-cc-notice-deny-btn", "body > div#cookieconsent > div#cookieconsent-bar > div:nth-child(3):not([id]) > a:nth-child(2)#decline", 'body > header#header > div:nth-child(1)#CookiesConsent > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(2):not([id]) > div:nth-child(5):not([id]) > button:not([id])[data-testid="AcceptRequiredCookies"]', "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookieChoiceInfo > div:nth-child(2)#cookieButtonBar > a:nth-child(2)#cookieChoiceRefuse", "body > dialog:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button#consent-banner-btn-close", "body > div#orejime > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > section#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cbgccp-cookies-banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#cbg_ccp_cookie_refuse_optional_btn", "body > div > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > button:nth-child(1)#fd-unCheckAll", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > dialog:not([id]) > article:not([id]) > footer:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#entry > div:nth-child(2)#main > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(3):not([id]) > dialog:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cb-cookie-warning > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cb-cookie-warning__button--decline", "body > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookies-banner > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2)#gdpr-popup-v3-button-mandatory", 'body > div:not([id]) > div:nth-child(1):not([id])[data-testid="privacy-banner"] > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#react-aria491930105-\\:r29\\:[data-testid="privacy-banner-decline-all-btn-desktop"]', "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:not([id]) > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", "body > div#tc-privacy-wrapper > div:nth-child(2)#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(3)#footer_tc_privacy_button_3", "body > div#app > div:nth-child(1)#__nuxt > div#__layout > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > a:nth-child(2):not([id])", "body > footer:not([id]) > div:nth-child(2):not([id]) > cookie-notice:nth-child(1):not([id]) > div#cookie-notice > div:not([id]) > div#cookie-notice-inner > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button#cookie-notice-decline", "body > div#sell-root > div:nth-child(2)#appRoot > div:nth-child(7):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(6):not([id]) > a:not([id])", "body > div:not([id]) > div:nth-child(1)#consent-manager > footer:nth-child(2):not([id]) > button:nth-child(2):not([id])", 'body > div#app > div:nth-child(7):not([id])[data-testid="cookieBanner"] > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])[data-testid="onlyNecessaryCookies"]', 'body > div:not([id]) > dialog:nth-child(2):not([id])[data-testid="modal-dialog"] > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2):not([id])[data-testid="uc-button-decline"]', "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", 'body > div#root > div:nth-child(2):not([id]) > div:not([id]) > div:not([id])[data-testid="cookie-consent-banner"] > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div#c-pop > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#ctl07_declinebtn", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id])", "body > div#cookieNoticePro > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookieReject", "body > div#gdrp-cookieoverlay > div:nth-child(3)#cs2gdpr-cookiebanner > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#btn-accept-required-banner", "body > div#seeGdprCookieConsent > div:not([id]) > p:nth-child(2):not([id]) > a:nth-child(2)#seeGdprReject", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(2)#popin_tc_privacy_button_2", "body > div#SgCookieOptin > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div#cookieman-modal > div:not([id]) > div:not([id]) > div:not([id]) > button:nth-child(6):not([id])", "body > div#c20343 > div:nth-child(1)#ikanos-privacy-cookielayer > div:nth-child(2):not([id]) > form:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#consent-banner-btn-close", "body > div#mm-2 > div:nth-child(2)#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:not([id]) > div:nth-child(2):not([id]) > main:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#application > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(6):not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(5):not([id]) > a:not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:not([id])", "body > dialog#ai-hinweis > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > center:not([id]) > button:nth-child(1)#policy_notwendig", "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > div#BannerRegion > div:nth-child(1)#Banner_cookie_0 > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#rejectAllBtn", "body > comply-consent-manager:not([id]) > div#comply-consent-manager > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", 'body > div#__next > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id])[data-testid="cookie_notice_reject_all_button"]', "body > div#tc-privacy-wrapper > div:nth-child(2)#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(2)#popin_tc_privacy_button_2", "body > div#__nuxt > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div#cdk-overlay-0 > mat-dialog-container:nth-child(2)#mat-mdc-dialog-0 > div:not([id]) > div:not([id]) > hra-consent-layer-ui:not([id]) > hra-cookie-buttons:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id]) > span:nth-child(2):not([id])", "body > div#cookie-note-main > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-accept-required", "body > aside:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > button:nth-child(4):not([id])", "body > div#cookiebanner-body > div:nth-child(1)#cookiebanner > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > div#app > div:nth-child(1)#header > div:nth-child(2)#cookie_banner > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#__nuxt > div#__layout > div#app > div:nth-child(6):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > section:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > section#shopify-pc__banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#shopify-pc__banner__btn-decline", "body > div#__nuxt > div#__layout > div:not([id]) > section:not([id]) > div:nth-child(2):not([id]) > section:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#consent-banner-btn-close", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:nth-child(2):not([id]) > a:not([id])", "body > div#tc-privacy-wrapper > div:nth-child(2)#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(3)#popin_tc_privacy_button_3", "body > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#modal-wrap > section#modal-content > div:not([id]) > div:nth-child(2)#cookie-banner > div:not([id]) > div:nth-child(1)#s1 > p:nth-child(5):not([id]) > a:nth-child(4)#button_reject", "body > div#cookie-consent-banner > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#btn-reject-all", "body > div#cc-button > div#cc-text > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#page > main:nth-child(2)#index > div:nth-child(3)#cookie-settings-modal > div:not([id]) > div#cookie-settings > div:nth-child(4):not([id]) > button:nth-child(3):not([id])", "body > div#__nuxt > div:nth-child(8)#cookie-banner > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", 'body > div#consentBanner > div:not([id]) > div#gdpr-banner-container[data-testid="gdpr-banner-container"] > dialog#gdpr-banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#gdpr-banner-cmp-button[data-testid="gdpr-banner-decline-button"]', "body > div#consentWidget > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#denyBtn", "body > div#portal-footer > div:nth-child(3):not([id]) > div:nth-child(2)#footer-analytics > div#CookieConsent > div:not([id]) > p:nth-child(2):not([id]) > span:nth-child(3):not([id]) > button:not([id])", "body > div#cookieman-modal > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "#tc-privacy-wrapper", '[data-testid="consentManager"]', '[data-testid="privacyBanner-rejectAll"]', "body > div#ww_bzga_matomo_cookiebanner > div:not([id]) > p:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#ddt-M1 > div:not([id]) > div:nth-child(1)#ddt-Seite1 > div:nth-child(2)#ddt-sectionFirst > p:nth-child(4):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div#cookies-bar > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", 'body > div:not([id]) > div:nth-child(2)#cookie-consent[data-testid="cookie-consent-banner"] > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])', "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(3)#popin_tc_privacy_button_3", "body > div#_evidon-barrier-wrapper > div:nth-child(2)#_evidon-banner > div:nth-child(1)#_evidon-banner-content > div:nth-child(5):not([id]) > button:nth-child(2)#_evidon-barrier-declinebutton", "div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > p:nth-child(2):not([id]) > a:not([id])", 'body > div#consent_modal[data-testid="Over18ModalVariant1Modal"] > div:nth-child(2):not([id])[data-testid="Over18ModalVariant1Content"] > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1)#reject-all-cookies-btn', "body > div:not([id]) > div:nth-child(1)#matomoCookieNotification > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#disallowMatomoCookieNotification", "body > div:not([id]) > aside:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#et-consent-overlay > div:nth-child(5):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button:not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(6):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(1)#cookies-eu-banner > div:not([id]) > div:not([id]) > button:nth-child(4)#cookies-eu-reject", "body > div:not([id]) > div:not([id]) > div:not([id]) > ul:not([id]) > li:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#optInId > div:nth-child(4):not([id]) > button:nth-child(2)#Ablehnen", "body > div:not([id]) > div#cpnb > div#w357_cpnb_outer > div:not([id]) > div:nth-child(2):not([id]) > span:nth-child(2)#cpnb-decline-btn", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#a-page > div:nth-child(5)#sc-content-container > div:nth-child(1):not([id]) > div:nth-child(12):not([id]) > div:nth-child(1) > div:nth-child(2)#cookie-consent-window > div:not([id]) > div:nth-child(2) > div:nth-child(1)#cookie-consent-continue > div:not([id]) > a:not([id])", "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:not([id]) > div:not([id]) > main:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id])", "body > div#acris--page-wrap--cookie-permission > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-permission--accept-only-functional-button", "body > div#cookieConsentBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#consentMinimal", "body > div#elGuestTerms > div:not([id]) > div:nth-child(2):not([id]) > form:not([id]) > button:nth-child(3):not([id])", "body > div#mm-0 > div:nth-child(2)#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id])", "body > div#cookiebar > div:not([id]) > span:nth-child(3)#declineCookie > button:not([id])", "body > main#content > aside:nth-child(10):not([id]) > div:not([id]) > wm-stack:nth-child(2):not([id]) > wm-button:nth-child(1):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(1)#footer_tc_privacy_container_text > div#footer_tc_privacy_text > h2:nth-child(1):not([id]) > button#footer_tc_privacy_button_2", "body > div#bandeau_cgv > div:not([id]) > div:not([id]) > a:nth-child(1)#dismiss-cookies", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > section:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > article:not([id]) > header:nth-child(2)#header > div:nth-child(1)#cookie-modal > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookieRefuse", "body > div#cookie-banner > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(3)#reject-cookies", "body > div:not([id]) > div:not([id]) > div:not([id]) > div#P0-0 > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > div#ppms_cm_consent_popup_7a8e4f13-52de-496c-9239-96129f02cf08 > div#ppms_cm_popup_overlay > div#ppms_cm_popup_wrapper > div:nth-child(1)#ppms_cm_popup > div:nth-child(3)#ppms_cm_popup_main_id > div:nth-child(2)#ppms-124c44a9-52c6-4136-9964-9a6be955271a > button:nth-child(2)#ppms_cm_disagree", "body > div#cookie > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > a:nth-child(2):not([id]) > span:not([id]) > span:not([id])", "body > div#orejime > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > div#cookie-message > a:nth-child(2):not([id])", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(1)#popin_tc_privacy_container_text > div#popin_tc_privacy_text > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#refuse_all", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#main > div:nth-child(11)#cookbar_overlay > div#cookbar > span:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > a:nth-child(3):not([id])", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(1)#popin_tc_privacy_button", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > main#main > dialog:nth-child(25):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div#cookie-banner > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:not([id])", 'body > div:not([id]) > div:nth-child(1):not([id])[data-testid="privacy-banner"] > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#react-aria526766221-\\:r0\\:[data-testid="privacy-banner-decline-all-btn-desktop"]', "body > div:not([id]) > form:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button#cookie_consent_use_only_functional_cookies", "body > div#cookiesplus-modal-container > div:not([id]) > div:nth-child(1)#cookiesplus-modal > div:nth-child(3)#cookiesplus-content > div:not([id]) > form#cookiesplus-form > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > app-root:not([id]) > div:not([id]) > app-cookie-banner:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2)#reject-cookie-link > button#reject-cookie", "body > main:not([id]) > footer:nth-child(4)#footer > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3)#acb-banner > div:nth-child(2)#acb-action > button:nth-child(1)#acb-deny-all-button", "body > div#st-cmp-v2 > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > span:not([id]) > div:not([id])", "body > div#cookie-consent > div:nth-child(1):not([id]) > button:nth-child(1)#cookie-deny-button", "body > div#advencyRgpd > button:nth-child(3)#refuseAll", "body > div#rgpd > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#consent > user-consent:nth-child(1):not([id]) > consent-dialog:not([id]) > consent-content:not([id]) > consent-message:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div#bottom-banner > div:nth-child(3):not([id]) > div:nth-child(1)#cookies-win > button:nth-child(2):not([id])", 'body > div:not([id]) > div:not([id])[data-testid="cookie-consent-banner"] > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(8):not([id]) > a:not([id])", "body > div#___gatsby > div:nth-child(1)#gatsby-focus-wrapper > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div#privacy-banner > div:nth-child(3):not([id]) > button:nth-child(1)#privacy-rejected", "body > div#cookie > div:not([id]) > div:nth-child(2)#Buttondivanalytic > button:nth-child(2):not([id])", "body > div:not([id]) > p:nth-child(2):not([id]) > button:nth-child(2)#seopress-user-consent-close", "body#top > div:not([id]) > form:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-notice > div:not([id]) > span:nth-child(2)#cn-notice-buttons > button:nth-child(2)#cn-refuse-cookie", "body:not([id]) > div#BannerRegion > div:nth-child(1)#Banner_cookie_0 > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#rejectAllBtn", "body:not([id]) > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", "body#html-body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#modal-root > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id]) > div:not([id])", "body:not([id]) > div#root > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-banner-container > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookie-law > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(12):not([id])", "body:not([id]) > div#__next > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#root > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > header#header > div:nth-child(1)#CookiesConsent > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(2):not([id]) > div:nth-child(5):not([id]) > button:not([id])", "body:not([id]) > div#__next > div:not([id]) > div:nth-child(6)#cw-footer-container > footer:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id]) > span:nth-child(2):not([id]) > span:nth-child(1):not([id])", "body > div#cookie_alert > div:nth-child(2)#cookie_alert_container > div#cookie_alert_text > div:nth-child(2):not([id]) > div:nth-child(2)#js-cookie_alert_button_decline > a#cookie_alert_decline", "body > div > div:not([id]) > div:nth-child(1) > div:nth-child(2)#cookieDisclaimer > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:not([id])", "body:not([id]) > dialog:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body:not([id]) > dialog:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body#unlogged-body > div#cookie-banner-deezer > div#gdpr-dir-tag > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#gdpr-btn-refuse-all", "body:not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > section#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#cc--main > div#cc_div > div:nth-child(1)#cm > div#c-inr > div:nth-child(2)#c-bns > button:nth-child(1)#c-s-bn", "body > div#headlessui-portal-root > div:not([id]) > div:nth-child(2):not([id]) > div#headlessui-dialog-_r_0_ > div:nth-child(2):not([id]) > div#headlessui-dialog-panel-_r_6_ > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body:not([id]) > div#cookie-wall > div#cwi > div:nth-child(4)#cw-controls > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2)#cwc-reject > span:not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#sliding-popup > div#cookiepopup > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#elGuestTerms > div:not([id]) > div:nth-child(2):not([id]) > form:not([id]) > button:nth-child(3):not([id])", "body:not([id]) > div#consent-box > p:not([id]) > span:not([id]) > button:nth-child(2)#decline-cookies", "body > div#__next > div:nth-child(3)#cookieBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > form:nth-child(2):not([id]) > button:nth-child(2)#cookieConsentRejectAll", "body:not([id]) > section:not([id]).privacy-banner > div:not([id]).privacy-banner__wrap > div:not([id]).privacy-content > div:not([id]).privacy-banner__grid-wrap > div:not([id]).privacy-banner__grid-col.privacy-banner__inner > div:nth-child(3):not([id]).privacy-banner__actions.privacy-banner__set.privacy-banner__btn-wrapper--stacked > button:nth-child(2):not([id]).privacy-banner__btn.privacy-banner__btn--secondary.privacy-banner__reject", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#confirmSelection", "body:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2)#gdpr-popup-v3-button-mandatory", "body#hgcBody > div#cookies-banner > a:nth-child(6)#btnCookieBannerNo", "body:not([id]) > dialog#biccy-banner > div#biccy-prompt > div:nth-child(2):not([id]) > button:nth-child(2)#biccy-reject-button", "body:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div > div:nth-child(1) > div:nth-child(3) > div:nth-child(2)", "body > div > div:nth-child(1) > div:nth-child(1) > div:nth-child(2) > button:nth-child(3)", "body > aside#cookies-policy > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#cookies > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#cookiesReject", "body > div:not([id]) > aside:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2)#modal-content-17 > div#pr-cookie-notice > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#btn-cookie-decline", "body > div:not([id]) > div:nth-child(14):not([id]) > div#m-cookienotice > div:not([id]) > div:nth-child(3)#action-custom-css > a:nth-child(2):not([id])", "body > div#__nuxt > div#__layout > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#dux-privacy > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#__nuxt > div#__layout > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "[data-testid='reject-button']", "body > div > div:nth-child(10) > div:nth-child(2) > div > div:nth-child(2) > div:nth-child(2)", "body > div#shopui-cookie-popup-container > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > section:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#__nuxt > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookies_overlay > div#cookies_banner > div:nth-child(2)#cookies_decision > button:nth-child(2)#cookies_read_declined", "body:not([id]) > div#__next > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#cookie-consent-popup > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-reject", "body > div#cookie-consent-banner > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#btn-reject-all", "body > div:not([id]) > div:nth-child(78)#cookie-banner > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cribbon > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#select-only-necessary > span:not([id])", "body > div#app > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#cookieconsent > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1)#cookie-deny", "body > div#modal-cookiesettings > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:nth-child(1)#react-content > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div#cookie-bar-2019 > div:nth-child(4):not([id]) > a#declineLink", 'body > div#__next > div:nth-child(2)#cookieBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])[data-testid="refuse-all-cookies"]', "body > aside#cookie-consent-banner > div:not([id]) > form:not([id]) > div:nth-child(2):not([id]) > fieldset:nth-child(3):not([id]) > ul:not([id]) > li:nth-child(1):not([id]) > button:not([id])", "body > dialog:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > footer:nth-child(3):not([id]) > div:nth-child(3):not([id]) > span:not([id])", "body > div#cookie-popup > div:nth-child(2):not([id]) > button:nth-child(2)#reject-cookies", "body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", 'div[data-test-consent-banner-popup] button[aria-label="Niet accepteren"]', "body > dialog:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:nth-child(2):not([id]) > a:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(1)#react-content > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#consent-modal > div:not([id]) > div:nth-child(1)#consent-intro > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#consent-deny-all", "body > div:not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookielaw_banner > div:nth-child(2):not([id]) > a:nth-child(2)#cookielaw_reject", "body > div#__next > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > mini-profiler:not([id]) > div:nth-child(1)#nlportal > div:nth-child(2)#nlportal-cookie-consent > div:not([id]) > div:nth-child(1):not([id]) > section:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#_app > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#radix-_r_0_ > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div#nimbu-consent > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(2):not([id]) > button:nth-child(2):not([id])", '.ind-cbar[data-testid="ind-cbar"]', '.ind-cbar[data-testid="ind-cbar"] button[save-cookie][data-value="0"]', "body > div:not([id]) > div#container > section:nth-child(1):not([id]) > div:not([id]) > div#block-mumc-info-cookiesui > div#cookiesjsr > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > web-overlay:not([id]) > web-cookie-consent:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#wpca-lay-out-wrapper > div:nth-child(1)#wpca-bar > div:nth-child(2)#wpca-bar-meta > button:nth-child(3):not([id])", 'body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cookiebanner-decline[data-testid="button-cookie-decline"]', "body > div#cookiebanner > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > span:nth-child(2):not([id]) > a:nth-child(2)#cookie-niet-akkoord", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > section#cookie-consent > div:nth-child(7):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1)#button-\xABr0\xBB", "body > aside#cookies-policy > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > form:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > a:nth-child(4):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(7)#modals > div:not([id]) > dialog:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#cookies-eu-wrapper > div#cookies-eu-banner > div:nth-child(2)#cookies-eu-buttons > button:nth-child(1)#cookies-eu-reject", "body > div#wrapper > footer:nth-child(3):not([id]) > div#footer > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#cc-card > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > a:nth-child(2)#cc-dismiss-btn", "body > div#__nuxt > div:not([id]) > div:nth-child(3):not([id]) > section#modal > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-bar > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div#CNID_4eb59999-7a35-450d-a08f-1661b81e5bc4 > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#refuseAll", "body > div#__nuxt > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(8)#modals > div:not([id]) > dialog:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > button:nth-child(2):not([id])", 'body > div:not([id]) > div:nth-child(3)#cookie-consent[data-testid="cookie-consent-banner"] > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])', "body > div#_rootSiteLayout > aside:nth-child(5):not([id]) > div:nth-child(3):not([id]) > aside:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > footer:not([id]) > div:nth-child(2)#cookieParent > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#noCookies", "body > div#siteWrapper > div:nth-child(1):not([id]) > section:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2)#bcSubmitConsentToNone", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > button:nth-child(4):not([id])", "body > shn-dialog#firstTimeVisitorCookieDialog > dialog:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > section#cc-window-overlay > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#cookieconsent > section:not([id]) > div:not([id]) > p:nth-child(6):not([id]) > a:not([id])", "body > ticketswap-portal:not([id]) > ul:not([id]) > div:not([id]) > div:not([id]) > span:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2)#ez-cookie-notification > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#ez-cookie-notification__decline", "body > div#__nuxt > div:nth-child(2)#__layout > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2)#cookie-consent > div:not([id]) > dialog:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div#cookie-consent-modal-description > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > section#chakra-modal-\\:rd\\: > div#chakra-modal--body-\\:rd\\: > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > form:not([id]) > button:nth-child(2)#cookie_settings_disallowed", "body > div#app > div:not([id]) > div:nth-child(10):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(1)#cookie-banner > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3)#cookie-banner-reject", "body > main#maincontent > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#svid10_17c33ae315e94c3ae6647bf > div:nth-child(1)#svid12_3f29801717d5618f8df9bf4 > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(4)#afCookieDecline", "body > div#CookieConsent > dialog:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cc-b-custom", "body > aza-app:not([id]) > aza-shell:not([id]) > div:nth-child(2):not([id]) > aza-cookie-message:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div#app > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#geotargetlygeoconsent1749551372435container > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#ppms_cm_consent_popup_d0973bf6-9332-4ce3-bc7c-09678b958daf > div#ppms_cm_popup_overlay > div:nth-child(2)#ppms_cm_popup_wrapper > div#ppms_cm_popup > div#ppms_cm_popup_main_id > div#ppms_cm_popup_responsive_wrapper_id > div:nth-child(2)#ppms-4ad6f5ba-7508-4caa-bd23-e8e9a4bd64b2 > div:nth-child(2)#ppms-85b001a4-b886-4f7e-82d5-bd34f50f3266 > button:nth-child(2)#ppms_cm_reject-all", "body > div#__nuxt > div:nth-child(2)#main-wrapper > div:nth-child(5):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#shopify-section-sections--20055816765655__privacy-banner > cookie-bar:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > form:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookie-consent-modal > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(4):not([id])", "body > div#CookieConsent > dialog:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cc-b-custom", "body > div#app > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#__docusaurus > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#rcc-decline-button", "body > div#CookieConsent > dialog:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button#cc-b-custom", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#radix-_r_2_ > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > section#component-cookie-banner > div:nth-child(1):not([id]) > footer:nth-child(4):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2)#cookie-banner-accept-essentials", "body > form#cookieConsentForm > div#cookie-consent-modal > div:not([id]) > div#cookie-consent-modal-content > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > form:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div#cookie_banner > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > container:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div#versionized-cookie-banner > div:nth-child(2):not([id]) > ul:not([id]) > li:nth-child(2):not([id]) > button#btn-reject-cookies", "body:not([id]) > div#consent-banner > div:nth-child(2)#truste-consent-track > div:nth-child(3)#truste-consent-content > div:nth-child(3):not([id]) > div#truste-consent-buttons > button:nth-child(3)#truste-consent-required", 'body > div#__next > div:not([id]) > div:nth-child(1)#cookieBanner[data-testid="cookie-banner"] > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:not([id]) > a:nth-child(2):not([id])', "body > div[class] > div:nth-child(2) > div:nth-child(2) > button:nth-child(2)", "body > div#cmpbox > div:not([id]) > div:nth-child(1)#cmpboxcontent > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > div[id][cookie-banner-data-theme] > div > div > div > div:nth-child(2) > div:nth-child(2) > button", "body:not([id]) > div#seers-cmp-cookie-data-hol > div:nth-child(2)#SeersCMPBannerMainBar > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3)#gdpr_cookie_info_bar-wr > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div[id][class][data-role][data-controller][style] > div > div:nth-child(2) > form > button:nth-child(3)", "body > div[class][tabindex] > div > div > div:nth-child(2) > div:nth-child(2) > button:nth-child(2)", "body > div[class][id][tabindex][role][aria-labelledby][aria-modal][style] > div > div > div:nth-child(3) > div > div > p > button:nth-child(3)", "body > div[id] > div:nth-child(2) > div:nth-child(4) > div > div:nth-child(2) > button:nth-child(2)", "body > div#cassie-widget > div:nth-child(4)#cassie_cookie_module > div:nth-child(2)#cassie_pre_banner > div:nth-child(3)#cassie_pre_banner__footer > button:nth-child(2)#cassie_reject_all_pre_banner", "body > div:not([id]) > div:nth-child(17):not([id]) > div:not([id]) > a:nth-child(3):not([id])", "body:not([id]) > div#__next > div:not([id]) > main:nth-child(3):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(27):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div#ch2-dialog > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div[class][role][aria-labelledby][aria-live][lang] > div:nth-child(1) > div:nth-child(3) > div:nth-child(2)", "body > div[id] > div > div:nth-child(1) > div > div > div > div:nth-child(2) > button:nth-child(2)", "body:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "#cookie-consent", '#cookie-consent[style*="max-height"] #cookie-status-reject', "body.enable-cookie-control", ".cookie-dialog-initial", ".cookie-dialog-initial button.g-button.outline", "cookie_preferences=%7B%22allow%22%3A%5B%5D", "#cookie_modal_placeholder dialog.bm-modal--cookie", "Analytics_consent=2", "Marketing_consent=2", '*[aria-labelledby="consent-banner-title"]', "#bbccookies-prompt", "#consent-banner-title", "#bbccookies-reject-button", 'button[data-testid="reject-button"]', "ckns_explicit=1", "#note .cpref", "#note .cpref #cPrefToggle", "#cPrefToggle", '#cPrefModal label[for="tab-4r"]', "#cPrefSave", ".modalwindow-container.bar .modalwindow", "#bibliotheek-nl-page", ".modalwindow-container.bar .modalwindow button.button.primary.notok", "div.modal.cookiesModal.is-open", 'div.cookiesModal__buttonWrapper > button[data-closecause="close-by-manage-cookies"]', "button#js-manage-data-privacy-save-button", "#cookie-notification", "#cookie-notification .cookie_pref_wrapper .pref-opt.refuse", ".incentive-banner", "#cookie-consent-banner .all4-cc-primary-button", '#cookie-consent-banner button[aria-label="Reject non-essential cookies and continue."]', "C4_CC=eyJ2ZXJzaW9uIjoxLCJjb25zZW50ZWQiOnRydWUs", "dialog[data-cookie-consent]:has(form[data-privacy-consent-form])", 'dialog[data-cookie-consent] form[data-privacy-consent-form] input[name="action"][value="reject"]', 'dialog[data-cookie-consent] form[data-privacy-consent-form]:has(input[name="action"][value="reject"]) button[type="submit"]', "dialog[data-cookie-consent]", "#gdpr-cookie-message", "[data-testid='cookie-banner-reject-button']", "CMCCP=AD%3D0", "#cookie-disclaimer", "#cookiesel", ".box-cookie", ".box-cookie .box-cookie__inner", ".box-cookie .btn-normal.is-small", "disney_cc=ok", 'div[class*="Overlay__container"]:has(div[class*="TCF2Popup"])', 'div[class*="TCF2Popup"]', '[class*="TCF2Popup"] a[href^="https://www.dailymotion.com/legal/cookiemanagement"]', 'button[class*="TCF2ContinueWithoutAcceptingButton"]', "dm-euconsent-v2", 'div[role="dialog"][class*="CustomCookieBanner"]', "#onetrust-consent-sdk .ot-pc-refuse-all-handler", "OptanonAlertBoxClosed", "ngc-cookie-banner", "div.cookie-footer-container", "[data-testid=cookieBanner]", "[data-testid=cookieBanner] button", "[data-testid=cookieBanner__manageCookiesButton]", "[data-testid=cookieModal] input[type=radio][value=false]:not(:checked):not(:disabled)", "[data-testid=cookieModal__acceptButton]", "gdpr__", ".cc-window.cc-visible", ".cc-window .cc-dialog", ".cc-window.cc-visible .cc-consent-require-only", "dji_consentmanager", "[id^=cookie-consent-banner]", "#cookie-consent-denied", "cookie-consent=denied", "#gdpr-banner", "#gdpr-banner-decline", ".cookie-wrapper", ".cookie-wrapper > .cookie-notice", "[data-test-id=cookie-notice-reject]", "#edp-cookies-banner", "#edp-cookies-banner .edp-cookies-refuse", "edp_cookie_agree=0", "#ef-ccpa", "#ef-button-ccpa-decline", ".modal:has(> .modal-content > .cookie-consent)", ".cookie-consent #cookieDisagree", "#es-cookie-banner", "#es-cookie-banner #es-consent-settings-btn", "#es-consent-settings-btn", "#onetrust-pc-sdk .save-preference-btn-handler", "#onetrust-pc-sdk", "#onetrust-consent-sdk input.category-switch-handler:checked", "div#klaro", "xpath///div[contains(@class,'pointer-events-none')]//button[normalize-space()='Accept all' or normalize-space()='Tout accepter']", "xpath///button[normalize-space()='Customize' or normalize-space()='Personnaliser']", "xpath///button[normalize-space()='Decline' or normalize-space()='Refuser']", "escaparium_cookie_consent_decided=1", ".cdk-overlay-container", ".cdk-overlay-container app-esaa-cookie-component", ".btn-cookie-refuser", ".cck-container", '.cck-actions-button[href="#refuse"]', "body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id])", '[data-testid="cookie-policy-manage-dialog"]', '[data-testid="cookie-policy-manage-dialog-decline-button"]', "#gdrp", ".cookie-consent", ".modal-cookie", ".cookies-modal", ".show-cookies-modal .cookies-modal #cookies-accept", ".show-cookies-modal .cookies-modal #cookies-reject", ".fixed [data-testid=closeCookieBanner]", "[data-testid=consent-banner]", "[data-testid=manage-preferences]", "[data-testid=consent-mgr-dialog] [data-ga-button=save-preferences]", 'div[role="dialog"][aria-label^="Privacy Disclosure"]', "#CookieConsent", "#CookieConsentDeclined", "#header-gdpr", "#header-gdpr #header-gdpr-in-btn button", "#header-gdpr.is-active", "gdprCookieData=ok", "[data-testid=consent-banner] [data-testid=reject-button]", `xpath///div[contains(., "Vill du till\xE5ta anv\xE4ndningen av cookies fr\xE5n Instagram i den h\xE4r webbl\xE4saren?") or contains(., "Allow the use of cookies from Instagram on this browser?") or contains(., "Povolit v prohl\xED\u017Ee\u010Di pou\u017Eit\xED soubor\u016F cookie z Instagramu?") or contains(., "Dopustiti upotrebu kola\u010Di\u0107a s Instagrama na ovom pregledniku?") or contains(., "\u0420\u0430\u0437\u0440\u0435\u0448\u0438\u0442\u044C \u0438\u0441\u043F\u043E\u043B\u044C\u0437\u043E\u0432\u0430\u043D\u0438\u0435 \u0444\u0430\u0439\u043B\u043E\u0432 cookie \u043E\u0442 Instagram \u0432 \u044D\u0442\u043E\u043C \u0431\u0440\u0430\u0443\u0437\u0435\u0440\u0435?") or contains(., "Vuoi consentire l'uso dei cookie di Instagram su questo browser?") or contains(., "Povoli\u0165 pou\u017E\xEDvanie cookies zo slu\u017Eby Instagram v tomto prehliada\u010Di?") or contains(., "Die Verwendung von Cookies durch Instagram in diesem Browser erlauben?") or contains(., "Sallitaanko Instagramin ev\xE4steiden k\xE4ytt\xF6 t\xE4ll\xE4 selaimella?") or contains(., "Enged\xE9lyezed az Instagram cookie-jainak haszn\xE1lat\xE1t ebben a b\xF6ng\xE9sz\u0151ben?") or contains(., "Het gebruik van cookies van Instagram toestaan in deze browser?") or contains(., "Bu taray\u0131c\u0131da Instagram'dan \xE7erez kullan\u0131m\u0131na izin verilsin mi?") or contains(., "Permitir o uso de cookies do Instagram neste navegador?") or contains(., "Permi\u0163i folosirea modulelor cookie de la Instagram \xEEn acest browser?") or contains(., "Autoriser l\u2019utilisation des cookies d\u2019Instagram sur ce navigateur ?") or contains(., "\xBFPermitir el uso de cookies de Instagram en este navegador?") or contains(., "Zezwoli\u0107 na u\u017Cycie plik\xF3w cookie z Instagramu w tej przegl\u0105darce?") or contains(., "\u039D\u03B1 \u03B5\u03C0\u03B9\u03C4\u03C1\u03AD\u03C0\u03B5\u03C4\u03B1\u03B9 \u03B7 \u03C7\u03C1\u03AE\u03C3\u03B7 cookies \u03B1\u03C0\u03CC \u03C4o Instagram \u03C3\u03B5 \u03B1\u03C5\u03C4\u03CC \u03C4\u03BF \u03C0\u03C1\u03CC\u03B3\u03C1\u03B1\u03BC\u03BC\u03B1 \u03C0\u03B5\u03C1\u03B9\u03AE\u03B3\u03B7\u03C3\u03B7\u03C2;") or contains(., "\u0420\u0430\u0437\u0440\u0435\u0448\u0430\u0432\u0430\u0442\u0435 \u043B\u0438 \u0438\u0437\u043F\u043E\u043B\u0437\u0432\u0430\u043D\u0435\u0442\u043E \u043D\u0430 \u0431\u0438\u0441\u043A\u0432\u0438\u0442\u043A\u0438 \u043E\u0442 Instagram \u043D\u0430 \u0442\u043E\u0437\u0438 \u0431\u0440\u0430\u0443\u0437\u044A\u0440?") or contains(., "Vil du tillade brugen af cookies fra Instagram i denne browser?") or contains(., "Vil du tillate bruk av informasjonskapsler fra Instagram i denne nettleseren?")]`, "xpath///button[contains(., '\u041E\u0442\u043A\u043B\u043E\u043D\u0438\u0442\u044C \u043D\u0435\u043E\u0431\u044F\u0437\u0430\u0442\u0435\u043B\u044C\u043D\u044B\u0435 \u0444\u0430\u0439\u043B\u044B cookie') or contains(., 'Decline optional cookies') or contains(., 'Refuser les cookies optionnels') or contains(., 'Hylk\xE4\xE4 valinnaiset ev\xE4steet') or contains(., 'Afvis valgfrie cookies') or contains(., 'Odmietnu\u0165 nepovinn\xE9 cookies') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03C0\u03C1\u03BF\u03B1\u03B9\u03C1\u03B5\u03C4\u03B9\u03BA\u03CE\u03BD cookies') or contains(., 'Neka valfria cookies') or contains(., 'Optionale Cookies ablehnen') or contains(., 'Rifiuta cookie facoltativi') or contains(., 'Odbij neobavezne kola\u010Di\u0107e') or contains(., 'Avvis valgfrie informasjonskapsler') or contains(., '\u0130ste\u011Fe ba\u011Fl\u0131 \xE7erezleri reddet') or contains(., 'Recusar cookies opcionais') or contains(., 'Optionele cookies afwijzen') or contains(., 'Rechazar cookies opcionales') or contains(., 'Odrzu\u0107 opcjonalne pliki cookie') or contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043D\u0435 \u043D\u0430 \u0431\u0438\u0441\u043A\u0432\u0438\u0442\u043A\u0438\u0442\u0435 \u043F\u043E \u0438\u0437\u0431\u043E\u0440') or contains(., 'Odm\xEDtnout voliteln\xE9 soubory cookie') or contains(., 'Refuz\u0103 modulele cookie op\u0163ionale') or contains(., 'A nem k\xF6telez\u0151 cookie-k elutas\xEDt\xE1sa')]", "form.rodo-popup", "form.rodo-popup button.rodo-popup-main-settings", "form.rodo-popup button.rodo-popup-reject", ".pop-cookie", "#cookiebanner-popup", "#jg-chrome-header", "#cookiebanner-popup #accept-essential-Cookies", "necessary:true%2Cpreferences:false", "#react-consent-modal", "#react-consent-modal .flex.flex-column", "#react-consent-modal .flex.flex-row.justify-between button:first-child", ".cookie:has(.cookie__inner .cookie__button)", ".cookie > .cookie__inner > .cookie__button > a", "#loader.is--show", ".cookie", "cookiesubmit=1", "#gdpr-banner-container", "#gdpr-banner-container #gdpr-banner [data-testid=gdpr-banner-decline-button]", "div:has(> div > a[href*='/privacy-policy'])", '[class*="CookiesBanner_cookies-banner"]', '[class*="CookiesBanner_cookies-banner"] [class*="CookiesBanner_buttons"] button', '[class*="CookiesBanner_buttons"] button:first-child', '[role="dialog"] [class*="_actions_"] button:first-child', "%22marketing%22%3Afalse", "#cookiePopup", ".cookie-overlay", "#cookiePopup .reject-cookie", "_site_acceptance=0", '.cookie-banner[data-block-name="cookie-banner"]', '.cookie-banner[data-block-name="cookie-banner"] .alert-close', 'section[aria-label="Cookie preferences"]', 'xpath///section[@aria-label="Cookie preferences"]//button[normalize-space()="Cookie Settings"]', 'div[role="dialog"][aria-labelledby="consent-settings-title"]', 'xpath///label[contains(., "Necessary and functional cookies")]//input[@name="cookie-tier"]', 'div[role="dialog"][aria-labelledby="consent-settings-title"] button:not([aria-label])', 'div:has(> div > div > div[role=alert] > a[href^="https://policy.medium.com/medium-privacy-policy-"])', "#cookie-container", 'div[aria-label="Cookie Policy Banner"]', "dialog.cookieconsent__dialog", '.cookieconsent__base[data-component="cookieconsent"]', ".cookieconsent__buttonAcceptNecessary", "consent_choice=", ".cookie-modal.open", ".cookie-modal.open .cookie-box", ".cookie-modal.open .btn-box .simple-btn.cancel", "#cmp-banner-sdk", ".cmp-sdk-container", "#cmp-reject-all-handler", "#onetrust-banner-sdk", "div#cookieWarning", "a#btnCookiesDenyAll", "[class*=ConsentManager]", "[class*=ConsentManager_cookieBar]", 'xpath///button[contains(@class, "ConsentManager_cookieBarButton") and contains(., "weigeren")]', "[data-testid=cookie-dialog-root]", "input[type=radio][id$=-declineLabel]", "[data-testid=confirm-choice-button]", "ccm-notification", "gdpr-banner", ".cookies-agreement-notification,.modal-new:has([data-module=SetupCookies])", ".cookies-agreement-notification", ".cookies-agreement-notification .cb_setup", "[data-module=SetupCookies]", "[data-module=SetupCookies] input[type=checkbox]:checked:not(:disabled)", "[name=button_save_choice]", "div.b-cookies-informer", "div.b-cookies-informer__nav > button:nth-child(1)", "div.b-cookies-informer__switchers", "div.b-cookies-informer__switchers input:not([disabled])", "div.b-cookies-informer__nav > button", "[aria-labelledby=cookieConsentTitle]", "xpath///button[contains(., 'Reject non-essential')]", "consent=rejected", "#cookie-consent .cookie-all__btn", "#cookie-consent .cookie-consent__switch.active:not(.always_on)", "#cookie-consent .cookie-selection__btn", "cookie_consent_essential=true", "cookie_consent_marketing=true", ".cookie-manager", ".cookie-manager .cookie-notice.open", ".cookie-notice button.owid-btn--outline-dark-blue.cookie-notice__button", "footer .ccpabanner", "#cookiePrefsModal, .privacy-sheet-content", "#cookiePrefsModal #formContent, .privacy-sheet-content #formContent", "#formContent input[type='checkbox']:checked", ".cookieAction #submitCookiesBtn", ".BusinessCookieConsent", ".BusinessCookieConsent [data-id=cookie-consent-banner-buttons]", "[data-id=cookie-consent-banner-buttons] > div:nth-child(2) button", "#cookie-consent button", "#cookie-consent input[type=checkbox]:checked:not(:disabled)", "plosCookieConsentStatus=false", "#cookieBanner.cbShort", "#cookieBanner.cbShort .cbCloseButton", "#cookieBanner #cookieBannerContent", "#cookieBanner #cookieBannerContent, #globalCookieBanner", "#globalCookieBanner .js-customizeGlobalCookies", "#cookieBanner [data-label=accept_essential]", "#cookieBanner button.cbSecondaryCTA", "#cookie-form", "#cookie-form .reject", "_cookieNoticeSettings=%7B%22performance%22%3Afalse%2C%22targeting%22%3Afalse%2C%22analytic%22%3Afalse%7D", "pnl-cookie-wall-widget", "CookiePermissionInfo", ".alert .accept-cookies,form.js-cookies", ".alert:has(.accept-cookies)", "form.js-cookies button", "form.js-cookies input[type=checkbox]", "form.js-cookies input[type=checkbox]:checked:not(:disabled)", "form.js-cookies button[type=submit]", ".alert:has(.accept-cookies) a[href='/account/cookies']", ".c-modal.is-active", ".c-modal.is-active .is-dismiss", "#__rptl-cookiebanner #__rptl-cookiebanner-reject", ".DialogHandlerContainer.visible:has(.cookie-dialog)", "#cookiesStrip:has([data-testid='accept-cookies'])", ".DialogHandlerContainer.visible:has(.cookie-dialog), #cookiesStrip:has([data-testid='accept-cookies'])", "#cookiesStrip [data-testid='skip-cookies']", "span.cookie-option-large>div>span.link-no-decoration.gui-text", ".cookie-dialog form .CookieConsentOption input[type='checkbox']:not([name='permanent']):checked", ".cookie-dialog button[type='submit']", "[bundlename=reddit_cookie_banner]", "#data-protection-consent-dialog rpl-modal-card > button.button-primary", "#data-protection-consent-dialog rpl-modal-card > button.button-secondary", "eu_cookie={%22opted%22:true%2C%22nonessential%22:false}", "div:has(> button#cookieBar-button)", "#cookieBar-button", ".cookie-banner-wrapper", ".cookie-banner-wrapper .cookie-banner", ".cookie-banner-wrapper button.btn-secondary-lg", "RBXcb", "#cookie-policy-info", 'div.cookie-btn-box > div[aria-label="Reject"]', '.cookie-policy-lightbox-bottom > div[aria-label="Save Settings"]', "#m-cookienotice", "#manage-cookies", "#accept-selected", "rspb-cookie-banner", "rspb-cookie-banner .buttons button.btn-light", 'rspb-cookie-banner [data-testid="options"]', 'rspb-cookie-banner button[label="Save settings"]', "RSPB.AllowMarketingCookies=No", ".cookies-banner-shown", "#ConsentBar", '#ConsentBar [data-js--cookie="cookies-accept-functional"]', "#gdprconsent.gdprcontainer", "#gdprconsent .gdprbutton a", "gdpr-consent=0", "#cookie-popup-with-overlay", "#cookie-popup-with-overlay [data-ref='cookie.no-thanks']", "RY_COOKIE_CONSENT", "div.cookie-bar", "body > div#__next > div#app > div:nth-child(7):not([id])", '[data-qa="GDPRBanner-Container"]', '[data-qa="GDPRBanner-Container"] [data-qa="GDPRBanner-Button"]', ".fc-consent-root .fc-dialog-container", '[class*="cookie-consent-required"]', 'html[data-cookie-consent-required] [data-testid="accept-all-cookie-button"]', '[data-testid="accept-all-cookie-button"]', "body[data-scroll-locked]", '[data-testid="customize-cookie-button"]', '[data-testid="confirm-settings-cookie-button"]', "%22analytics%22%3Afalse", "body > div[class*=_shein_privacy]", "body > div[class*=_shein_privacy] > div:nth-of-type(2) > div:nth-of-type(4) > div:nth-of-type(3)", "custom-cookie-consent#cookieBubble.banner-show", "custom-cookie-consent#cookieBubble #configureBtn", "custom-cookie-consent#cookieBubble.banner-show #configureBtn", ".custom-cookie-modal #acceptNecessaryBtn", '[data-testid="cookie-wall"]', '[data-testid="cookie-wall"] .cookie-bar__footer button[data-id="0"]', "acceptCookies=0", "#cookieBannerContent", "[data-tracking-element-id=cookie_banner_essential_only]", "app-cookie-consent", "app-cookie-consent button.underline", "app-cookie-consent ui-switch button.switch", "app-cookie-consent ui-switch button.switch.checked:not(.disabled)", "app-cookie-consent app-btn button", ".consent__wrapper", ".consent", "button.consentSettings", "button#consentSubmit", "[class*=CookieConsent__root___]", "[class*=CookieConsent__modal___]", "[class*=CookieConsent__modal___] > div > button[class*=secondary]:nth-child(2)", 'cookie-consent-1={"optedIn":true,"functionality":false,"statistics":false}', 'div > div > div > div > span[href*="/cookie-and-similar-technologies-policy.html"]', "xpath///span[contains(., 'Alle afwijzen') or contains(., 'Reject all') or contains(., 'T\xFCm\xFCn\xFC reddet') or contains(., 'Odrzu\u0107 wszystko')]", 'div > div > div:has(> div > span[href*="/cookie-and-similar-technologies-policy.html"]) > [role=button]:nth-child(2)', "[aria-label=consent-banner]", "xpath///button[contains(., 'Reject all')]", "interactionCount=1", "#cookie_banner", "#tsla-reject-cookie", "tsla-cookie-consent=rejected", "div[id='sp_message_container_1482251'],div[id='sp_message_container_1482252']", "html.sp-message-open", "#consent-banner", "#consent-banner button[data-testid='close-button']", "[data-test-id=cookies_section]", "#cookie-consent #cookie-consent-btn-customise", "#cookie-consent #cookie-consent-btn-accept-some", "#cookie-consent-btn-customise", "#consent-marketing:checked", "#consent-marketing", "#consent-analytics:checked", "#consent-analytics", "#cookie-consent-btn-accept-some", "%22analytics%22%3Afalse%2C%22marketing%22%3Afalse", "xpath///div[contains(@class, 'fixed') and contains(., 'Cookie\u3092\u30D6\u30ED\u30C3\u30AF') and .//button[normalize-space()='Cookie\u306E\u8A2D\u5B9A']]", "xpath///div[contains(@class, 'fixed') and contains(., 'Cookie\u3092\u30D6\u30ED\u30C3\u30AF')]//button[normalize-space()='Cookie\u306E\u8A2D\u5B9A']", "xpath///button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']", ".optIn", ".optIn .js-optin-cookie-settings", 'label[for="TAB-02"]', 'label[for="TAB-02"] + .tab-content .toggleSwitch__circle', 'label[for="TAB-03"]', 'label[for="TAB-03"] + .tab-content .toggleSwitch__circle', ".js-modal-save-settings", "__performanceCookieFlag__=0", "__targetingCookieFlag__=0", "div:has(> button#accept-button):has(> button#decline-button)", "button#accept-button,button#decline-button", "button#decline-button", "#consent-modal", "#privacy-settings-content", 'button[type="submit"]', "div.one-modal__action-footer-column--secondary > a", ".fixed.bottom-0:has([data-test=cookieBannerButton])", ".fixed.bottom-0 [data-test=cookieBannerButton]", '[data-a-target="consent-banner"],.ReactModalPortal:has([data-a-target=consent-modal-save])', '[data-a-target="consent-banner"]', '[data-a-target="consent-banner-accept"]', '[data-a-target="consent-banner"] button:not([data-a-target])', 'button[data-a-target="consent-banner-manage-preferences"]', "input[type=checkbox][data-a-target=tw-checkbox]:not([disabled])", "input[type=checkbox][data-a-target=tw-checkbox]:not(:checked):not([disabled])", "[data-a-target=consent-modal-save]", ".ReactModalPortal:has([data-a-target=consent-modal-save])", '[data-testid="BottomBar"]', "body > div#react-root > div:not([id]) > div:not([id]) > div:nth-child(1)#layers > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id])[data-testid=BottomBar] > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", ".cookie-banner", "#__tealiumGDPRcpPrefs #consent-container", "#__tealiumGDPRcpPrefs #consent-container #consent-modal-settings", "#no-consent-popup-close-modal", ".ucb", ".ucb-banner", ".ucb-banner .ucb-btn-save", "#js-pw-consent-wrapper bgl-modal[test-id='cookie-modal']", "#js-pw-consent-wrapper [data-ta-id='pw-consent-rejectAllButton']", ".gdpr_container", ".gdpr_modal_container", ".gdpr .gdpr_manage_cookies_button", ".gdpr .gdpr_container", ".gdpr_modal .gdpr_modal_button_reject", 'vg_consent={"functionality":false,"performance":false,"targeting":false}', ".cookie-policy-modal", ".manage-cookies-modal", ".cookie-policy-modal .cookie-policy-details button.manage", ".manage-cookies-modal .manage-cookies-footer button.btn-outline-primary", "%22analytics%22:false", ".dip-consent,.dip-consent-container", ".dip-consent-container", ".dip-consent-content", '.dip-consent-btn[tabindex="2"]', "cookie-banner#cookie-banner-host", "groups=C1%3A1%2CC3%3A0", 'a[href="https://www.walmartcanada.ca/cookie-notice"]', 'button[data-dca-intent="open"][role="link"]', "xpath///button[contains(., 'Save settings') or contains(., 'Enregistrer les param\xE8tres')]", "[class^=cookie_wrapper]", 'dialog[aria-labelledby="consent-title"]', "div.animate-slideFromLeft.fixed.bottom-0.z-50", 'dialog[aria-labelledby="consent-title"], div.animate-slideFromLeft.fixed.bottom-0.z-50', 'dialog[aria-labelledby="consent-title"] button.border-xe-primary-500, div.animate-slideFromLeft button.border-xe-primary-500', '[data-testid="performance-toggle"]', 'dialog[aria-labelledby="consent-title"] button.bg-xe-primary-500, div.animate-slideFromLeft button.bg-xe-primary-500', "xeConsentState={%22performance%22:false%2C%22marketing%22:false", '[data-role="cookies-modal"]', '[data-role="cookies-modal"] [data-opt-hydration="cookies-dialog-eu"]', '[data-role="cookies-modal"] [class*=experimentalButtons] button:nth-of-type(2)', '[data-role="cookies-modal"] [class*=container]', "[class*=cookiesAnnounce]", "[class*=cookiesAnnounce] [class*=announceText]", "div.cookie.flex.flex-wrap", "xpath///div[contains(@class,'cookie')]//button[normalize-space()='Deny']", "#consent-page", "#consent-page button[value=reject]", "#cookie_modal_wrapper", "#cookie_modal_wrapper #cookie_modal_button_choose", "#consent-init", "#consent-init #consent-configure", "#consent-update #consent-configuration-save", "zinio-cookie-consent"], r: [[1, "abconcerts.be", 2, "", 22, [0], [{ e: 1 }], [{ v: 1 }], [{ if: { e: 2 }, then: [{ k: 2 }], else: [{ k: 3 }, { c: 4 }] }], [], { intermediate: false }], [1, "acris", 2, "", 22, [5], [{ e: 6 }], [{ v: 7 }], [{ check: "any", wv: 8 }, { wait: 500 }, { c: 8 }], [], {}], [1, "adopt", 2, "", 22, [9], [{ e: 10 }], [{ v: 9 }], [{ c: 11 }], [{ eval: "EVAL_ADOPT_TEST" }], {}], [1, "Adroll", 2, "", 22, [12], [{ e: 12 }], [{ v: 12 }], [{ c: 13 }], [{ negated: true, cc: 14 }], {}], [1, "affinity.serif.com", 2, "", 22, [], [{ e: 15 }], [{ v: 16 }], [{ k: 17 }], [{ wait: 500 }, { cc: 18 }, { negated: true, cc: 19 }], {}], [1, "amazon.com", 2, "", 22, [20], [{ e: 20 }], [{ check: "any", v: 21 }], [{ wv: 22 }, { wait: 5e3 }, { k: 22 }], [], {}], [1, "amex", 0, "", 22, [23], [{ e: 23 }], [{ v: 23 }], [{ c: 24 }], [], {}], [1, "anthropic", 2, "", 22, [], [{ e: 25 }], [{ v: 25 }], [{ c: 26 }], [{ cc: 27 }], {}], [1, "AppConsent", 2, "", 22, [28], [{ e: 29 }], [{ v: 28 }], [{ waitForThenClick: ["iframe[title='Consent window']", ".button__refuseAll"] }], [{ timeout: 1e3, check: "none", wv: 28 }], {}], [1, "AppConsent legacy", 2, "", 22, [30], [{ e: 31 }], [{ v: 30 }], [{ waitForThenClick: ["iframe[srcdoc*='frame-root']", ".button__skip"] }], [{ timeout: 1e3, check: "none", wv: 30 }], {}], [1, "aquasana.com", 2, "", 22, [32], [{ v: 32 }], [{ v: 32 }], [{ if: { e: 33 }, then: [{ k: 33 }], else: [{ h: 32 }] }], [], {}], [1, "arbeitsagentur", 2, "", 22, [34], [{ e: 35 }], [{ e: 36 }], [{ waitForThenClick: ["bahf-cookie-disclaimer-dpl3", "#bahf-cookie-disclaimer-modal .ba-btn-contrast"] }], [{ cc: 37 }], {}], [1, "as-adventure", 2, "", 22, [38, 39, 40], [{ e: 41 }], [{ check: "any", v: 42 }], [{ all: true, retry: 10, retryInterval: 500, c: 41 }], [{ timeout: 2e3, check: "none", wv: 42 }], {}], [1, "asus", 2, "", 22, [43], [{ e: 44 }], [{ v: 44 }], [{ if: { e: 45 }, then: [{ k: 45 }], else: [{ c: 46 }, { c: 47 }] }], [], {}], [1, "automattic-cmp-optout", 2, "", 22, [48], [{ e: 48 }], [{ v: 48 }], [{ k: 49 }, { all: true, c: 50 }, { k: 51 }], [], {}], [1, "aws.amazon.com", 2, "", 22, [52, 53, 54, 55], [{ e: 52 }], [{ v: 52 }], [{ if: { e: 56 }, then: [{ k: 56 }], else: [{ k: 57 }, { w: 58 }, { all: true, optional: true, k: 59 }, { k: 60 }] }], [], {}], [1, "axeptio", 2, "", 22, [61], [{ e: 61 }], [{ any: [{ e: 62 }, { v: 63 }, { visible: [".axeptio_mount .needsclick", ".axeptio_widget"] }] }], [{ if: { e: 64 }, then: [{ waitForVisible: [".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton"] }, { wait: 300 }, { click: [".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton"] }], else: [{ wv: 65 }, { wait: 300 }, { k: 65 }] }], [{ cc: 66 }], {}], [1, "aylo-cookie-banner", 2, "", 22, [67], [{ e: 68 }, { e: 69 }], [{ v: 68 }, { any: [{ v: 70 }, { eval: "EVAL_AYLO_COOKIE_MANAGER_READY" }] }], [{ if: { v: 71 }, then: [{ c: 71 }], else: [{ if: { v: 72 }, then: [{ wait: 8e3 }, { c: 72 }], else: [{ c: 73 }] }] }, { optional: true, c: 74 }], [{ any: [{ cc: 75 }, { cc: 76 }] }], {}], [1, "b-cookie", 1, "", 22, [77], [{ e: 77 }], [{ v: 77 }], [{ h: 77 }], [], {}], [1, "baden-wuerttemberg.de", 2, "", 22, [78], [{ e: 78 }], [{ v: 78 }], [{ if: { e: 79 }, then: [{ k: 79 }], else: [{ all: true, optional: true, k: 80 }, { k: 81 }] }], [], {}], [1, "bbb.org", 0, "", 22, [82], [{ e: 83 }], [{ v: 84 }], [{ wv: 85 }, { wait: 500 }, { k: 85 }, { w: 86 }, { all: true, optional: true, k: 87 }, { k: 88 }, { c: 89 }], [], {}], [1, "bbva", 2, "", 22, [90], [{ e: 91 }], [{ v: 91 }], [{ c: 91 }, { check: "none", wv: 92 }], [{ cc: 93 }], {}], [1, "bigcommerce-consent-manager", 0, "", 22, [94], [{ e: 94 }, { eval: "EVAL_BIGCOMMERCE_CONSENT_MANAGER_DETECT" }], [{ v: 95 }], [{ c: 96 }], [{ cc: 97 }], {}], [1, "bing.com", 2, "", 22, [98], [{ e: 99 }], [{ v: 99 }, { v: 100 }], [{ wait: 500 }, { c: 101 }], [{ cc: 102 }], {}], [1, "blocksy", 0, "", 10, [103], [{ e: 104 }], [{ v: 103 }], [{ c: 105 }], [{ cc: 106 }], { intermediate: false }], [1, "borlabs", 2, "", 22, [107], [{ e: 108 }], [{ v: 109 }], [{ if: { e: 110 }, then: [{ k: 110 }], else: [{ if: { e: 111 }, then: [{ k: 111 }], else: [{ if: { e: 112 }, then: [{ k: 112 }], else: [{ if: { e: 113 }, then: [{ k: 113 }, { wv: 114 }, { all: true, optional: true, k: 115 }, { k: 116 }, { wait: 500 }] }] }] }] }], [{ eval: "EVAL_BORLABS_0" }], {}], [1, "bswhealth", 0, "", 22, [117], [{ e: 117 }], [{ v: 118 }], [{ c: 119 }, { wv: 120 }, { all: true, optional: true, c: 121 }, { c: 118 }], [{ cc: 122 }], {}], [1, "bundesregierung.de", 2, "", 22, [123], [{ e: 123 }], [{ v: 124 }], [{ wait: 500 }, { c: 125 }], [{ cc: 126 }], {}], [1, "burpee.com", 2, "", 22, [127, 128], [{ e: 128 }], [{ v: 129 }], [{ if: { e: 130 }, then: [{ c: 130 }, { all: true, k: 131 }, { k: 132 }], else: [{ h: 133 }] }], [], {}], [1, "cassie", 0, "", 10, [134], [{ e: 135 }], [{ v: 136 }], [{ c: 137 }], [], {}], [1, "cc_banner", 1, "", 22, [138], [{ e: 138 }], [{ v: 139 }], [{ h: 138 }], [], {}], [1, "cc-banner-springer", 2, "", 22, [140], [{ e: 140 }], [{ v: 140 }], [{ if: { e: 141 }, then: [{ k: 141 }], else: [{ c: 142 }, { w: 143 }, { all: true, optional: true, k: 144 }, { if: { e: 145 }, then: [{ k: 145 }], else: [{ k: 146 }] }] }], [{ eval: "EVAL_CC_BANNER2_0" }], {}], [1, "check24", 2, "", 22, [147], [{ e: 148 }], [{ v: 149 }], [{ c: 148 }], [{ check: "none", v: 147 }], {}], [1, "check24-partnerprogramm-de", 2, "", 22, [150], [{ e: 151 }], [{ check: "any", v: 151 }], [{ c: 152 }], [], {}], [1, "ciaopeople.it", 2, "", 22, [153], [{ e: 153 }], [{ v: 153 }], [{ c: 154 }, { w: 155 }, { c: 156 }], [{ check: "none", v: 153 }], {}], [1, "civic-cookie-control", 2, "", 22, [157], [{ e: 158 }], [{ v: 159 }, { v: 158 }], [{ if: { v: 160 }, then: [{ c: 160 }], else: [{ if: { exists: ["#ccc #ccc-notify .ccc-notify-buttons", "xpath///button[contains(., 'Settings') or contains(., 'Cookie Preferences') or contains(., 'Einstellungen\uFE0F')]"] }, then: [{ waitForThenClick: ["#ccc #ccc-notify .ccc-notify-buttons", "xpath///button[contains(., 'Settings') or contains(., 'Cookie Preferences') or contains(., 'Einstellungen\uFE0F')]"] }, { wv: 161 }] }, { if: { v: 160 }, then: [{ c: 160 }], else: [{ c: 162 }] }] }], [], {}], [1, "ckies", 2, "", 22, [163], [{ e: 163 }], [{ v: 163 }], [{ c: 164 }], [{ cc: 165 }], {}], [1, "click.io", 2, "", 22, [166], [{ e: 166 }], [{ v: 166 }], [{ w: 167 }, { wait: 500 }, { k: 167 }, { w: 168 }, { all: true, k: 168 }, { k: 169 }], [{ cc: 170 }], {}], [1, "clinch", 2, "", 10, [171], [{ e: 171 }], [{ v: 171 }], [{ if: { e: 172 }, then: [{ k: 172 }], else: [{ k: 173 }, { all: true, optional: true, k: 174 }, { k: 175 }] }], [{ cc: 176 }], { intermediate: false }], [1, "cloudflare-zaraz", 2, "", 22, [177], [{ exists: [".cf_modal_container", ".cf_consent-buttons"] }], [{ visible: [".cf_modal_container", ".cf_consent-buttons"] }], [{ waitForThenClick: [".cf_modal_container", ".cf_consent-buttons #cf_consent-buttons__reject-all"] }], [], {}], [1, "Complianz banner", 2, "", 22, [178], [{ e: 179 }], [{ check: "any", v: 179 }], [{ c: 180 }], [{ cc: 181 }], {}], [1, "Complianz categories", 2, "", 22, [182], [{ e: 182 }], [{ v: 182 }], [{ if: { e: 183 }, then: [{ k: 184 }], else: [{ all: true, optional: true, k: 185 }, { k: 186 }] }], [], {}], [1, "Complianz notice", 1, "", 22, [187], [{ e: 188 }], [{ v: 188 }], [{ if: { e: 189 }, then: [{ k: 189 }], else: [{ h: 190 }] }], [], {}], [1, "Complianz opt-out", 2, "", 22, [191], [{ e: 191 }], [{ v: 191 }], [{ timeout: 2e3, optional: true, w: 192 }, { if: { e: 189 }, then: [{ k: 189 }], else: [{ if: { e: 193 }, then: [{ k: 193 }, { c: 194 }, { c: 195 }] }] }], [], {}], [1, "Complianz optin", 2, "", 22, [196], [{ e: 196 }], [{ v: 196 }], [{ if: { v: 189 }, then: [{ k: 189 }], else: [{ if: { v: 197 }, then: [{ c: 197 }, { wv: 198 }, { all: true, optional: true, k: 199 }, { k: 200 }], else: [{ k: 184 }] }] }], [], {}], [1, "consent-flo", 2, "", 22, [201], [{ e: 202 }], [{ v: 203 }], [{ if: { e: 204 }, then: [{ c: 204 }], else: [{ c: 205 }, { wv: 206 }, { all: true, optional: true, k: 207 }, { c: 206 }] }], [{ e: 208 }], {}], [1, "consentmanager-ncmp", 2, "", 22, [209], [{ e: 210 }], [{ check: "any", v: 210 }], [{ wait: 500 }, { eval: "EVAL_CONSENTMANAGER_NCMP_REJECT" }], [{ cc: 211 }], {}], [1, "consentmo", 2, "", 22, [212], [{ exists: ["csm-cookie-consent", ".csm-wrapper"] }], [{ visible: ["csm-cookie-consent", ".csm-wrapper button"] }], [{ if: { exists: ["csm-cookie-consent", ".csm-wrapper .cc-deny"] }, then: [{ waitForThenClick: ["csm-cookie-consent", ".csm-wrapper .cc-deny"] }], else: [{ waitForThenClick: ["csm-cookie-consent", ".csm-wrapper .cc-settings"] }, { waitForThenClick: ["csm-cookie-consent", ".csm-wrapper .cc-deny"] }] }], [{ waitForVisible: ["csm-cookie-consent", ".csm-wrapper button"], timeout: 1e3, check: "none" }], {}], [1, "Cookie Information Banner", 2, "", 22, [213], [{ e: 213 }], [{ v: 213 }], [{ eval: "EVAL_COOKIEINFORMATION_0" }, { wait: 1e3 }, { if: { v: 214 }, then: [{ h: 213 }] }], [{ cc: 215 }], {}], [1, "cookie-banner-element", 2, "", 22, [216], [{ e: 216 }], [{ visible: ["cookie-banner-element#ckb", '#ckb[role="dialog"]'] }], [{ waitForThenClick: ["cookie-banner-element#ckb", "#ckn"] }], [], {}], [1, "cookie-consent-spice", 0, "", 10, [217, 218], [{ e: 218 }], [{ v: 218 }], [{ c: 219 }], [], {}], [1, "cookie-law-info", 2, "", 22, [220], [{ e: 221 }, { eval: "EVAL_COOKIE_LAW_INFO_DETECT" }], [{ v: 221 }], [{ h: 220 }, { eval: "EVAL_COOKIE_LAW_INFO_0" }], [{ negated: true, cc: 222 }], {}], [1, "cookie-manager-popup", 0, "", 10, [223], [{ e: 224 }], [{ v: 128 }], [{ if: { e: 225 }, then: [{ k: 225 }], else: [{ c: 223 }, { wv: 226 }, { all: true, optional: true, k: 227 }, { k: 228 }] }], [{ eval: "EVAL_COOKIE_MANAGER_POPUP_0" }], { intermediate: false }], [1, "cookie-script", 2, "", 22, [229], [{ e: 229 }], [{ v: 229 }], [{ if: { e: 230 }, then: [{ wait: 100 }, { k: 230 }], else: [{ k: 231 }, { wv: 232 }, { c: 230 }] }], [], {}], [1, "cookieacceptbar", 1, "", 22, [233], [{ e: 233 }], [{ v: 233 }], [{ h: 233 }], [], {}], [1, "cookiebot.be", 2, "", 22, [234], [{ e: 235 }], [{ v: 234 }], [{ c: 235 }], [{ cc: 236 }], {}], [1, "cookieconsent2", 2, "", 22, [237], [{ e: 237 }], [{ e: 238 }, { e: 239 }], [{ if: { e: 240 }, then: [{ c: 240 }], else: [{ c: 241 }, { wait: 500 }, { c: 242 }] }], [{ cc: 243 }], {}], [1, "cookieconsent3", 2, "", 22, [244], [{ e: 244 }], [{ v: 245 }], [{ if: { e: 246 }, then: [{ c: 246 }], else: [{ c: 247 }, { c: 248 }] }], [{ cc: 243 }], {}], [1, "cookiecuttr", 0, "", 10, [249], [{ e: 250 }], [{ v: 250 }], [{ if: { e: 251 }, then: [{ k: 251 }], else: [{ h: 249 }] }], [], {}], [1, "cookiefirst.com", 2, "", 22, [252], [{ e: 253 }], [{ check: "any", v: 254 }], [{ if: { e: 255 }, then: [{ k: 255 }], else: [{ if: { e: 256 }, then: [{ timeout: 5e3, wv: 256 }, { k: 256 }, { wait: 1500 }], else: [{ if: { e: 257 }, then: [{ k: 257 }, { timeout: 1e3, wv: 258 }, { eval: "EVAL_COOKIEFIRST_1" }, { wait: 1e3 }, { k: 259 }], else: [{ k: 255 }] }] }] }], [{ eval: "EVAL_COOKIEFIRST_0" }], {}], [1, "cookiehub", 2, "", 22, [260], [{ e: 261 }], [{ v: 261 }], [{ if: { e: 262 }, then: [{ k: 262 }, { wv: 263 }, { if: { e: 264 }, then: [{ k: 264 }], else: [{ all: true, optional: true, k: 265 }, { k: 266 }] }], else: [{ h: 260 }] }], [{ cc: 267 }], {}], [1, "cookieinfo", 1, "", 22, [268], [{ e: 268 }], [{ v: 268 }], [{ c: 269 }], [], {}], [1, "cookiejs-banner", 2, "", 22, [270], [{ e: 271 }], [{ v: 272 }], [{ c: 273 }], [{ cc: 274 }], {}], [1, "cookiejs-modal", 2, "", 22, [270], [{ e: 275 }], [{ v: 276 }], [{ c: 277 }], [{ cc: 278 }], {}], [1, "cookieyes", 2, "", 22, [279], [{ e: 280 }], [{ v: 280 }], [{ if: { e: 281 }, then: [{ c: 281 }], else: [{ if: { e: 282 }, then: [{ c: 282 }, { w: 283 }, { all: true, optional: true, k: 284 }, { c: 285 }, { optional: true, wv: 286 }, { optional: true, c: 287 }], else: [{ if: { e: 288 }, then: [{ k: 288 }, { w: 289 }, { all: true, optional: true, k: 290 }, { c: 291 }], else: [{ h: 292 }] }] }] }], [{ cc: 293 }], {}], [1, "coolblue", 0, "", 10, [294], [{ e: 295 }], [{ v: 296 }], [{ c: 297 }], [], {}], [1, "corona-in-zahlen.de", 2, "", 22, [298], [{ e: 298 }], [{ v: 298 }], [{ k: 299 }, { k: 300 }], [], {}], [1, "ct-ultimate-gdpr", 2, "", 22, [301], [{ e: 301 }], [{ timeout: 3e4, wv: 301 }], [{ if: { v: 302 }, then: [{ k: 302 }], else: [{ if: { v: 303 }, then: [{ k: 303 }, { c: 304 }, { k: 305 }], else: [{ h: 301 }] }] }], [{ wait: 500 }, { cc: 306 }], {}], [1, "curseforge", 1, "", 22, [307], [{ e: 308 }], [{ v: 308 }], [{ h: 307 }], [], {}], [1, "dailymotion-us", 1, "", 22, [309], [{ e: 310 }], [{ v: 310 }], [{ h: 310 }], [], {}], [1, "dan-com", 2, "", 10, [], [{ e: 311 }], [{ v: 311 }], [{ c: 312 }], [], {}], [1, "datagrail", 2, "", 22, [313], [{ e: 313 }, { exists: [".dg-consent-banner", ".dg-app"] }], [{ v: 313 }], [{ if: { exists: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some"] }, then: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some"] }], else: [{ if: { exists: [".dg-consent-banner", ".dg-browser-signal-notice"] }, then: [{ if: { exists: [".dg-consent-banner", "button.dg-button.open_layer"] }, then: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.open_layer"] }, { waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some, button.dg-button.essential_only, button.dg-button.custom"] }], else: [{ waitForThenClick: [".dg-consent-banner", "button.dg-header-close"] }] }], else: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.open_layer"] }, { waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some, button.dg-button.essential_only"] }] }] }], [{ any: [{ cc: 314 }, { exists: [".dg-consent-banner", ".dg-browser-signal-notice"] }] }], {}], [1, "didomi", 2, "", 22, [315], [{ e: 316 }], [{ check: "any", v: 317 }], [{ if: { e: 318 }, then: [{ c: 318 }], else: [{ eval: "EVAL_DIDOMI_OPT_OUT" }] }], [{ eval: "EVAL_DIDOMI_TEST" }], {}], [1, "dmgmedia", 2, "", 22, [319], [{ e: 320 }], [{ v: 320 }], [{ c: 321 }, { wv: 322 }, { all: true, c: 323 }, { waitForThenClick: ['[data-project="mol-fe-cmp"] [class*=footer]', "xpath///button[contains(., 'Save & Exit')]"] }], [], {}], [1, "dmgmedia-us", 2, "", 22, [324], [{ e: 325 }], [{ wv: 325 }], [{ c: 326 }, { wv: 327 }, { c: 328 }, { c: 329 }], [], {}], [1, "doordash-storefront", 2, "", 22, [330], [{ e: 330 }], [{ v: 330 }], [{ retry: 15, retryInterval: 300, c: 331 }], [{ cc: 332 }], {}], [1, "dpgmedia-nl", 2, "", 22, [333], [{ e: 333 }], [{ visible: ["#pg-root-shadow-host", "#pg-modal"] }], [{ waitForThenClick: ["#pg-root-shadow-host", "#pg-configure-btn"] }, { waitForThenClick: ["#pg-root-shadow-host", "#pg-reject-btn"] }], [], {}], [1, "dreamlab-cmp", 2, "", 22, [334], [{ e: 335 }], [{ v: 334 }], [{ c: 336 }, { c: 337 }], [{ cc: 338 }], {}], [1, "drouot-rgpd", 2, "", 22, [339, 340], [{ e: 341 }], [{ v: 339 }], [{ c: 342 }], [{ cc: 343 }], {}], [1, "Drupal", 2, "", 22, [], [{ e: 344 }], [{ v: 344 }], [{ k: 345 }], [], {}], [1, "dunelm.com", 2, "", 22, [346], [{ e: 347 }], [{ v: 347 }], [{ k: 348 }, { k: 349 }], [{ cc: 350 }, { cc: 351 }], {}], [1, "ecbeing", 1, "", 22, [352], [{ e: 353 }], [{ v: 354 }], [{ h: 354 }], [], {}], [1, "elsevier-pure", 2, "", 22, [355], [{ e: 356 }], [{ v: 356 }], [{ c: 357 }], [{ timeout: 1e3, check: "none", wv: 356 }], {}], [1, "Ensighten", 2, "", 22, [358, 359], [{ any: [{ v: 360 }, { v: 361 }] }], [{ any: [{ v: 360 }, { v: 361 }] }], [{ wait: 500 }, { if: { v: 360 }, then: [{ if: { check: "any", v: 362 }, then: [{ c: 362 }], else: [{ if: { check: "any", v: 363 }, then: [{ c: 363 }, { timeout: 1e3, w: 361 }, { all: true, optional: true, k: 364 }, { c: 365 }], else: [{ c: 366 }] }] }], else: [{ all: true, optional: true, k: 364 }, { c: 365 }] }], [], {}], [1, "epaas", 2, "", 22, [367], [{ e: 367 }], [{ visible: ["epaas-consent-drawer-shell", "section.consentDrawer"] }], [{ if: { exists: ["epaas-consent-drawer-shell", "button.reject-button"] }, then: [{ waitForThenClick: ["epaas-consent-drawer-shell", "button.reject-button"] }], else: [{ waitForThenClick: ["epaas-consent-drawer-shell", "button.personalize-button"] }, { waitFor: ["epaas-consent-drawer-shell", "epaas-policy-page-shell"] }, { waitForThenClick: ["epaas-consent-drawer-shell", "button.save-button"] }] }], [{ cc: 368 }], {}], [1, "etsy", 2, "", 22, [369, 370], [{ e: 369 }], [{ v: 369 }], [{ k: 371 }, { timeout: 3e3, wv: 372 }, { wait: 1e3 }, { eval: "EVAL_ETSY_0" }, { eval: "EVAL_ETSY_1" }], [], {}], [1, "EU Cookie Law", 1, "", 22, [373], [{ e: 374 }], [{ wait: 500 }, { v: 374 }], [{ h: 374 }], [{ negated: true, cc: 375 }], {}], [1, "eu-cookie-compliance-banner", 2, "", 22, [], [{ e: 376 }], [{ e: 376 }], [{ k: 377 }], [{ negated: true, cc: 378 }], {}], [1, "EZoic", 2, "", 22, [379], [{ e: 379 }], [{ v: 379 }], [{ wait: 500 }, { k: 380 }, { w: 381 }, { all: true, k: 382 }, { k: 383 }], [{ cc: 384 }], {}], [1, "fastcmp", 0, "", 22, [385], [{ e: 386 }], [{ v: 386 }], [{ waitForThenClick: ["iframe#fast-cmp-iframe", ".fast-cmp-home-refuse button"] }], [{ cc: 387 }], {}], [1, "fedex", 0, "", 22, [388], [{ e: 388 }], [{ v: 389 }], [{ c: 390 }], [{ cc: 391 }], {}], [1, "fever", 2, "", 22, [392], [{ e: 393 }], [{ v: 392 }], [{ c: 393 }, { if: { e: 394 }, then: [{ k: 395 }] }, { c: 396 }], [{ cc: 397 }], {}], [1, "fever-cookie-advice", 2, "", 22, [398], [{ e: 398 }], [{ v: 398 }], [{ c: 399 }, { wv: 400 }, { c: 401 }], [{ wait: 500 }, { cc: 402 }], {}], [1, "fides", 2, "", 22, [403], [{ e: 404 }], [{ v: 404 }, { eval: "EVAL_FIDES_DETECT_POPUP" }], [{ w: 405 }, { if: { v: 406 }, then: [{ k: 406 }], else: [{ k: 407 }, { c: 408 }] }], [], {}], [1, "finsweet", 0, "", 22, [409], [{ e: 409 }], [{ v: 410 }], [{ wait: 500 }, { if: { e: 411 }, then: [{ k: 411 }], else: [{ h: 409 }] }], [], {}], [1, "fullertonhotels.com", 0, "", 10, [], [{ exists: ["#ifrmCookieBanner", "#sp-decline"] }], [{ visible: ["#ifrmCookieBanner", "#sp-decline"] }], [{ wait: 500 }, { waitForThenClick: ["#ifrmCookieBanner", "#sp-decline"] }], [], {}], [1, "funding-choices", 2, "", 22, [412], [{ e: 413 }], [{ e: 414 }], [{ k: 415 }, { all: true, optional: true, k: 416 }, { optional: true, k: 417 }], [], {}], [1, "gallup", 2, "", 22, [418], [{ e: 419 }], [{ v: 419 }], [{ c: 419 }, { wait: 500 }, { c: 420 }], [{ cc: 421 }], {}], [1, "gdpr-legal-cookie", 2, "", 22, [422], [{ any: [{ eval: "EVAL_GDPR_LEGAL_COOKIE_DETECT_CMP" }, { e: 423 }] }], [{ check: "any", v: 424 }], [{ c: 425 }], [{ eval: "EVAL_GDPR_LEGAL_COOKIE_TEST" }], {}], [1, "gemini.google.com", 2, "", 22, [426], [{ e: 427 }], [{ v: 426 }], [{ c: 428 }], [], {}], [1, "godaddy-privacy-widget", 2, "", 22, [429], [{ exists: ["#gtm_privacy", "#privacy_widget"] }], [{ visible: ["#gtm_privacy", "#privacy_widget"] }], [{ waitForThenClick: ["#gtm_privacy", "#pw_decline"] }], [{ cc: 430 }], {}], [1, "google-consent-standalone", 2, "", 22, [], [{ e: 431 }, { e: 432 }], [{ v: 431 }], [{ c: 433 }], [], {}], [1, "google-cookiebar", 0, "", 22, [434], [{ e: 434 }], [{ v: 434 }], [{ if: { e: 435 }, then: [{ k: 435 }], else: [{ h: 434 }] }], [], {}], [1, "google.com", 2, "", 22, [436], [{ e: 436 }, { e: 437 }], [{ v: 438 }], [{ c: 438 }], [{ cc: 439 }], {}], [1, "gov.uk", 2, "", 22, [], [{ e: 440 }], [{ e: 441 }], [{ wait: 300 }, { if: { visible: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Reject')] | //a[contains(., 'Reject')] | //input[contains(@value, 'Reject')] | //button[contains(., 'do not use analytics cookies')]"] }, then: [{ click: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Reject')] | //a[contains(., 'Reject')] | //input[contains(@value, 'Reject')] | //button[contains(., 'do not use analytics cookies')]"] }] }, { waitForVisible: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Hide')] | //a[contains(., 'Hide')] | //input[contains(@value, 'Hide')]"] }, { click: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Hide')] | //a[contains(., 'Hide')] | //input[contains(@value, 'Hide')]"] }], [], {}], [1, "gravito", 2, "", 22, [442], [{ e: 442 }], [{ v: 442 }], [{ c: 443 }, { w: 444 }, { w: 445 }, { all: true, optional: true, k: 446 }, { c: 444 }], [{ cc: 447 }], {}], [1, "healthline-media", 2, "", 22, [448], [{ e: 449 }], [{ e: 449 }], [{ if: { e: 450 }, then: [{ k: 450 }], else: [{ wv: 451 }, { k: 452 }] }], [], {}], [1, "hema", 2, "", 22, [127], [{ v: 453 }], [{ v: 453 }], [{ c: 454 }], [{ cc: 455 }], {}], [1, "hl.co.uk", 2, "", 22, [456, 457], [{ e: 457 }], [{ e: 457 }], [{ k: 458 }, { h: 459 }, { w: 460 }, { optional: true, k: 461 }, { w: 462 }, { optional: true, k: 463 }, { k: 464 }], [], {}], [1, "holidaymedia", 2, "", 22, [465], [{ e: 466 }], [{ v: 465 }], [{ timeout: 2e3, c: 467 }], [], {}], [1, "hometogo", 2, "", 22, [468], [{ e: 469 }], [{ v: 468 }], [{ if: { e: 470 }, then: [{ c: 470 }], else: [{ c: 471 }, { c: 472 }] }], [{ cc: 473 }], {}], [1, "hu-manity", 2, "", 22, [474], [{ e: 475 }], [{ v: 475 }], [{ c: 476 }], [], {}], [1, "hubspot", 2, "", 22, [], [{ e: 477 }], [{ v: 477 }], [{ k: 478 }], [], {}], [1, "idxrcookies", 2, "", 22, [479], [{ e: 480 }], [{ v: 480 }], [{ c: 480 }], [], {}], [1, "infomaniak-rgpd", 2, "", 22, [481], [{ e: 481 }], [{ visible: ["module-rgpd-component", ".modal"] }], [{ waitForThenClick: ["module-rgpd-component", ".buttons > button:nth-of-type(2)"], retry: 3 }], [{ cc: 482 }], {}], [1, "inmobi", 2, "", 22, [483], [{ e: 484 }], [{ v: 484 }], [{ c: 485 }], [{ cc: 486 }], {}], [1, "ionos.de", 2, "", 22, [487, 488], [{ e: 488 }], [{ v: 488 }], [{ k: 489 }, { k: 490 }], [], {}], [1, "iubenda", 2, "", 22, [491], [{ e: 491 }], [{ v: 492 }], [{ if: { e: 493 }, then: [{ k: 493 }], else: [{ c: 494 }, { c: 495 }, { c: 496 }] }], [{ eval: "EVAL_IUBENDA_1" }], {}], [1, "iWink", 2, "", 22, [497], [{ e: 497 }], [{ v: 497 }], [{ c: 498 }], [{ cc: 499 }], {}], [1, "jetpack-eu-cookie-law", 1, "", 22, [500], [{ e: 500 }], [{ v: 500 }], [{ h: 500 }], [], {}], [1, "johnlewis.com", 2, "", 22, [501], [{ e: 501 }], [{ e: 501 }], [{ k: 502 }, { wait: 500 }, { all: true, optional: true, k: 503 }, { k: 504 }], [], {}], [1, "jquery.cookieBar", 1, "", 22, [505], [{ e: 506 }], [{ check: "any", v: 506 }], [{ h: 505 }], [{ check: "none", v: 506 }, { negated: true, cc: 507 }], {}], [1, "justwatch.com", 2, "", 22, [508], [{ e: 509 }], [{ v: 509 }], [{ k: 510 }, { c: 511 }, { all: true, c: 512 }, { all: true, optional: true, k: 513 }, { wv: 514 }, { k: 514 }], [], {}], [1, "kconsent", 0, "", 10, [515], [{ e: 516 }], [{ v: 517 }], [{ c: 518 }], [], {}], [1, "ketch", 2, "", 10, [519, 520, 521], [{ any: [{ e: 519 }, { e: 520 }, { e: 521 }, { e: 522 }] }], [{ any: [{ v: 519 }, { v: 520 }, { v: 521 }] }], [{ if: { e: 523 }, then: [{ c: 523 }, { if: { v: 524 }, then: [{ k: 525 }] }], else: [{ if: { e: 526 }, then: [{ c: 526 }], else: [{ if: { e: 527 }, then: [{ if: { e: 528 }, then: [{ c: 528 }], else: [{ if: { e: 529 }, then: [{ k: 529 }] }] }] }] }] }, { timeout: 1e3, optional: true, w: 530 }, { if: { e: 530 }, then: [{ if: { e: 531 }, then: [{ c: 531 }], else: [{ all: true, optional: true, k: 532 }] }, { k: 525 }] }], [{ cc: 533 }], { intermediate: false }], [1, "krown-cookie-banner", 0, "", 22, [534], [{ e: 535 }], [{ v: 535 }], [{ c: 535 }], [{ wait: 500 }, { eval: "EVAL_KROWN_COOKIE_BANNER_TEST" }], {}], [1, "lia", 2, "", 22, [536], [{ e: 536 }], [{ e: 537 }, { v: 536 }], [{ c: 538 }], [], {}], [1, "lightbox", 2, "", 22, [539], [{ e: 540 }], [{ v: 540 }], [{ k: 541 }], [], {}], [1, "lineagrafica", 1, "", 22, [542], [{ e: 542 }], [{ e: 542 }], [{ h: 542 }], [], {}], [1, "linkedin.com", 2, "", 22, [543], [{ e: 543 }], [{ v: 543 }], [{ wv: 544 }, { wait: 500 }, { c: 544 }], [{ check: "none", wv: 543 }], {}], [1, "macaron", 2, "", 22, [545], [{ e: 545 }], [{ v: 546 }], [{ if: { e: 547 }, then: [{ k: 547 }], else: [{ c: 548 }, { w: 549 }, { wv: 550 }, { all: true, optional: true, c: 551 }, { c: 552 }] }], [{ cc: 553 }], {}], [1, "mco-consent", 2, "", 22, [554, 555, 556, 557], [{ e: 558 }], [{ v: 559 }], [{ c: 559 }], [{ cc: 560 }], {}], [1, "mediamarkt.de", 2, "", 22, [561, 562], [{ e: 563 }], [{ e: 563 }], [{ k: 564 }], [], {}], [1, "mensaje-cookies", 2, "", 22, [565, 566], [{ e: 567 }, { any: [{ e: 568 }, { e: 569 }] }], [{ v: 570 }], [{ if: { e: 568 }, then: [{ c: 568 }], else: [{ c: 571 }, { c: 569 }] }, { check: "none", timeout: 3e3, wv: 570 }], [{ check: "none", v: 570 }], {}], [1, "microsoft.com", 2, "", 22, [572], [{ e: 572 }], [{ e: 572 }], [{ eval: "EVAL_MICROSOFT_0" }], [{ eval: "EVAL_MICROSOFT_2" }], {}], [1, "mirasvit-gdpr", 2, "", 22, [573, 574, 575, 576], [{ e: 577 }], [{ check: "any", v: 577 }], [{ if: { check: "any", v: 578 }, then: [{ any: [{ k: 579 }, { k: 580 }] }], else: [{ any: [{ k: 581 }, { k: 582 }] }, { check: "any", timeout: 5e3, wv: 583 }, { all: true, timeout: 2e3, optional: true, c: 584 }, { any: [{ k: 585 }, { k: 586 }, { k: 587 }] }] }, { check: "none", timeout: 1e4, wv: 577 }], [{ cc: 588 }, { check: "none", v: 577 }], {}], [1, "mkdocs-material", 2, "", 22, [589, 590], [{ e: 591 }], [{ v: 592 }], [{ all: true, optional: true, k: 593 }, { c: 594 }], [], {}], [1, "moneysavingexpert.com", 2, "", 22, [], [{ e: 595 }], [{ v: 595 }], [{ k: 596 }, { k: 597 }], [], {}], [1, "Moove", 2, "", 22, [598], [{ e: 598 }], [{ v: 599 }], [{ if: { e: 600 }, then: [{ k: 600 }], else: [{ if: { e: 601 }, then: [{ k: 601 }, { wv: 602 }, { eval: "EVAL_MOOVE_0" }, { k: 603 }], else: [{ h: 598 }] }] }], [{ check: "none", v: 598 }], {}], [1, "nhs.uk", 2, "", 22, [604], [{ e: 604 }], [{ e: 604 }], [{ k: 605 }], [], {}], [1, "obi.de", 2, "", 22, [606], [{ e: 607 }], [{ v: 607 }], [{ k: 608 }], [], {}], [1, "om", 2, "", 22, [609], [{ e: 610 }], [{ e: 610 }], [{ if: { e: 611 }, then: [{ c: 611 }], else: [{ all: true, optional: true, k: 612 }, { c: 613 }] }], [], {}], [1, "openli", 2, "", 22, [614], [{ e: 614 }], [{ check: "any", v: 615 }], [{ c: 616 }], [], {}], [1, "osano", 2, "", 22, [617], [{ e: 618 }], [{ eval: "EVAL_OSANO_DETECT" }, { v: 619 }], [{ if: { e: 620 }, then: [{ k: 620 }], else: [{ if: { v: 621 }, then: [{ k: 621 }], else: [{ h: 617 }] }] }], [], {}], [1, "otto.de", 2, "", 22, [622], [{ e: 622 }], [{ v: 623 }], [{ k: 624 }], [], {}], [1, "overleaf", 2, "", 22, [625], [{ e: 626 }], [{ v: 626 }], [{ c: 627 }], [{ cc: 628 }], {}], [1, "pabcogypsum", 2, "", 22, [629], [{ e: 630 }], [{ v: 630 }], [{ c: 631 }], [], {}], [1, "pandectes", 2, "", 22, [632], [{ e: 632 }], [{ v: 632 }], [{ if: { v: 633 }, then: [{ c: 633 }], else: [{ c: 634 }, { wv: 635 }, { k: 635 }, { wv: 636 }, { k: 636 }] }, { optional: true, h: 632 }], [{ wait: 500 }, { eval: "EVAL_PANDECTES_TEST" }], {}], [1, "paypal-us", 2, "", 22, [637], [{ e: 638 }], [{ v: 638 }], [{ if: { e: 639 }, then: [{ k: 639 }], else: [{ k: 640 }] }], [{ wait: 1e3 }, { cc: 641 }], {}], [1, "paypal.com", 2, "", 22, [642], [{ e: 642 }], [{ v: 643 }], [{ c: 639 }], [{ wait: 500 }, { cc: 644 }], {}], [1, "pikpak", 2, "", 22, [645], [{ e: 646 }], [{ v: 647 }], [{ c: 648 }, { wv: 649 }, { all: true, optional: true, k: 650 }, { c: 651 }], [{ any: [{ cc: 652 }, { cc: 653 }] }], {}], [1, "pmc", 1, "", 22, [654], [{ e: 654 }], [{ v: 654 }], [{ h: 654 }], [], {}], [1, "police-uk", 2, "", 22, [655], [{ e: 656 }], [{ v: 656 }], [{ c: 656 }, { timeout: 5e3, optional: true, wv: 657 }, { optional: true, k: 657 }], [{ cc: 658 }], {}], [1, "pornhat", 1, "", 22, [659], [{ v: 660 }], [{ v: 660 }], [{ h: 659 }], [], {}], [1, "pride.com", 1, "", 22, [661], [{ e: 662 }], [{ v: 662 }], [{ h: 661 }], [], {}], [1, "PrimeBox CookieBar", 2, "", 22, [663], [{ e: 664 }], [{ check: "any", v: 664 }], [{ optional: true, k: 665 }, { h: 663 }], [{ negated: true, cc: 666 }], {}], [1, "privado", 0, "", 10, [667], [{ e: 668 }], [{ v: 668 }], [{ if: { e: 669 }, then: [{ c: 669 }], else: [{ c: 670 }, { wv: 671 }, { c: 671 }] }], [], {}], [1, "pubtech", 2, "", 22, [672], [{ e: 672 }], [{ v: 673 }], [{ k: 674 }], [{ eval: "EVAL_PUBTECH_0" }], {}], [1, "quantcast", 2, "", 22, [675], [{ e: 676 }], [{ v: 677 }], [{ if: { e: 678 }, then: [{ k: 678 }], else: [{ timeout: 2e3, w: 679 }, { if: { e: 680 }, then: [{ k: 680 }], else: [{ k: 681 }, { wv: 682 }, { all: true, optional: true, k: 683 }, { click: [".qc-cmp2-main", "xpath///button[contains(., 'REJECT ALL') or contains(., 'ALLE VERWERPEN') or contains(., '\u0391\u03A0\u039F\u03A1\u03A1\u0399\u03A0\u03A4\u03A9 \u03A4\u0391 \u03A0\u0391\u039D\u03A4\u0391') or contains(., 'RESPINGERE TOTAL\u0102') or contains(., 'ALLE ABLEHNEN') or contains(., 'ODRZUCENIE') or contains(., 'BLOQUEAR TODO') or contains(., 'REJEITAR TODOS') or contains(., 'RIFIUTA TUTTO') or contains(., 'TOUT REFUSER') or contains(., '\u041E\u0422\u041A\u041B\u041E\u041D\u0418\u0422\u042C \u0412\u0421\u0415\u0425')]"], optional: true }, { wait: 500 }, { if: { e: 684 }, then: [{ k: 684 }], else: [{ waitForThenClick: [".qc-cmp2-main", "xpath///button[contains(.,'SAVE & EXIT') or contains(.,'SALVA ED ESCI') or contains(.,'GUARDAR Y SALIR') or contains(.,'SPEICHERN & VERLASSEN')"], timeout: 5e3 }] }] }] }], [], {}], [1, "r42-cookiebar", 0, "", 10, [685], [{ e: 686 }], [{ v: 687 }], [{ c: 686 }], [{ timeout: 1e3, check: "none", wv: 686 }], {}], [1, "rdc-concents", 2, "", 22, [688, 689], [{ e: 690 }], [{ v: 690 }], [{ c: 690 }], [{ negated: true, e: 691 }], {}], [1, "real-cookie-banner", 0, "", 10, [692], [{ e: 693 }], [{ v: 693 }], [{ waitForThenClick: ['div[consent-skip-blocker="1"][id][data-bg] > dialog > div > div > div > div > div > a[role=button]:not([id])', "xpath///span[contains(., ' ohne ') or contains(., 'without') or contains(., 'Ablehnen')]"] }], [{ timeout: 1e3, check: "none", wv: 693 }], {}], [1, "ring", 2, "", 22, [], [{ e: 694 }], [{ v: 694 }], [{ c: 695 }, { wv: 696 }, { all: true, optional: true, k: 697 }, { c: 698 }], [], {}], [1, "sandhills", 1, "", 22, [699], [{ e: 700 }], [{ v: 700 }], [{ h: 699 }], [], {}], [1, "sas", 2, "", 22, [701], [{ e: 701 }], [{ v: 701 }], [{ c: 702 }], [{ cc: 703 }], {}], [1, "setupad", 2, "", 22, [704], [{ e: 705 }], [{ v: 705 }], [{ if: { e: 706 }, then: [{ c: 706 }], else: [{ c: 707 }, { wv: 708 }, { all: true, optional: true, k: 709 }, { c: 706 }] }, { check: "none", wv: 704 }], [{ cc: 710 }], {}], [1, "shopify", 0, "", 22, [711], [{ e: 711 }], [{ v: 711 }], [{ c: 712 }], [{ eval: "EVAL_SHOPIFY_TEST" }], {}], [1, "sibbo", 2, "", 22, [713], [{ e: 713 }], [{ v: 714 }], [{ k: 714 }], [], {}], [1, "Sirdata", 0, "", 22, [715], [{ e: 715 }], [{ v: 715 }], [{ if: { exists: ["#sd-cmp", "xpath///button[contains(., 'Do not accept') or contains(., 'Acceptera inte') or contains(., 'No aceptar') or contains(., 'Ikke acceptere') or contains(., 'Nicht akzeptieren') or contains(., '\u041D\u0435 \u043F\u0440\u0438\u0435\u043C\u0430\u043C') or contains(., '\u039D\u03B1 \u03BC\u03B7\u03BD \u03B3\u03AF\u03BD\u03B5\u03B9 \u03B1\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE') or contains(., 'Niet accepteren') or contains(., 'Nep\u0159ij\xEDmat') or contains(., 'Nie akceptuj') or contains(., 'Nu accepta\u021Bi') or contains(., 'N\xE3o aceitar') or contains(., 'Continuer sans accepter') or contains(., 'Non accettare') or contains(., 'Nem fogad el')]"] }, then: [{ waitForThenClick: ["#sd-cmp", "xpath///button[contains(., 'Do not accept') or contains(., 'Acceptera inte') or contains(., 'No aceptar') or contains(., 'Ikke acceptere') or contains(., 'Nicht akzeptieren') or contains(., '\u041D\u0435 \u043F\u0440\u0438\u0435\u043C\u0430\u043C') or contains(., '\u039D\u03B1 \u03BC\u03B7\u03BD \u03B3\u03AF\u03BD\u03B5\u03B9 \u03B1\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE') or contains(., 'Niet accepteren') or contains(., 'Nep\u0159ij\xEDmat') or contains(., 'Nie akceptuj') or contains(., 'Nu accepta\u021Bi') or contains(., 'N\xE3o aceitar') or contains(., 'Continuer sans accepter') or contains(., 'Non accettare') or contains(., 'Nem fogad el')]"] }], else: [{ if: { exists: ["#sd-cmp", "xpath///span[contains(., '\u0417\u0430\u0434\u0430\u0439\u0442\u0435 \u0432\u0430\u0448\u0438\u0442\u0435 \u0438\u0437\u0431\u043E\u0440\u0438') or contains(., 'Nastavit va\u0161e volby') or contains(., 'Angiv dine valg') or contains(., 'Ihre Auswahl treffen') or contains(., '\u039F\u03C1\u03AF\u03C3\u03C4\u03B5 \u03C4\u03B9\u03C2 \u03B5\u03C0\u03B9\u03BB\u03BF\u03B3\u03AD\u03C2 \u03C3\u03B1\u03C2') or contains(., 'Set your choices') or contains(., 'Establecer preferencias') or contains(., 'Param\xE9trer vos choix') or contains(., 'V\xE1lassza ki a be\xE1ll\xEDt\xE1sokat') or contains(., 'Imposta le tue scelte') or contains(., 'Stel uw keuzes in') or contains(., 'Ustaw swoje wybory') or contains(., 'Definir suas escolhas') or contains(., 'Seta\u021Bi-v\u0103 op\u021Biunile') or contains(., 'St\xE4ll in dina val')]"] }, then: [{ waitForThenClick: ["#sd-cmp", "xpath///span[contains(., '\u0417\u0430\u0434\u0430\u0439\u0442\u0435 \u0432\u0430\u0448\u0438\u0442\u0435 \u0438\u0437\u0431\u043E\u0440\u0438') or contains(., 'Nastavit va\u0161e volby') or contains(., 'Angiv dine valg') or contains(., 'Ihre Auswahl treffen') or contains(., '\u039F\u03C1\u03AF\u03C3\u03C4\u03B5 \u03C4\u03B9\u03C2 \u03B5\u03C0\u03B9\u03BB\u03BF\u03B3\u03AD\u03C2 \u03C3\u03B1\u03C2') or contains(., 'Set your choices') or contains(., 'Establecer preferencias') or contains(., 'Param\xE9trer vos choix') or contains(., 'V\xE1lassza ki a be\xE1ll\xEDt\xE1sokat') or contains(., 'Imposta le tue scelte') or contains(., 'Stel uw keuzes in') or contains(., 'Ustaw swoje wybory') or contains(., 'Definir suas escolhas') or contains(., 'Seta\u021Bi-v\u0103 op\u021Biunile') or contains(., 'St\xE4ll in dina val')]"] }, { waitForThenClick: ["#sd-cmp", "xpath///span[contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043C \u0432\u0441\u0438\u0447\u043A\u043E') or contains(., 'Odm\xEDtnout v\u0161e') or contains(., 'Afvis alt') or contains(., 'Alle ablehnen') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03CC\u03BB\u03C9\u03BD') or contains(., 'Reject all') or contains(., 'Rechazar todo') or contains(., 'Tout refuser') or contains(., 'Mind elutas\xEDt\xE1sa') or contains(., 'Rifiuta tutto') or contains(., 'Alles weigeren') or contains(., 'Odrzu\u0107 wszystkie') or contains(., 'Rejeitar todos') or contains(., 'Respinge\u021Bi toate') or contains(., 'Avvisa allt')]"] }], else: [{ c: 716 }] }] }], [], {}], [1, "snigel", 2, "", 22, [], [{ e: 717 }], [{ v: 717 }], [{ k: 718 }, { k: 719 }], [{ cc: 720 }], {}], [1, "socialfunders", 2, "", 22, [721], [{ e: 722 }, { e: 723 }], [{ v: 721 }], [{ c: 722 }], [], {}], [1, "squarespace-cookie-banner", 2, "", 22, [724], [{ e: 725 }], [{ v: 725 }], [{ c: 726 }], [{ timeout: 1e3, check: "none", wv: 725 }], {}], [1, "squiz", 0, "", 10, [667], [{ e: 727 }], [{ v: 727 }], [{ c: 728 }], [{ wait: 500 }, { cc: 729 }], {}], [1, "steampowered.com", 2, "", 22, [], [{ e: 730 }, { v: 730 }], [{ v: 730 }], [{ k: 731 }], [{ wait: 1e3 }, { eval: "EVAL_STEAMPOWERED_0" }], {}], [1, "stright", 2, "", 22, [732, 733], [{ e: 732 }], [{ v: 732 }], [{ if: { v: 734 }, then: [{ c: 734 }], else: [{ c: 735 }, { c: 736 }] }, { check: "none", wv: 732 }], [{ check: "none", v: 732 }], {}], [1, "stripchat.com", 0, "", 22, [737], [{ e: 738 }], [{ v: 738 }], [{ c: 739 }, { c: 740 }], [{ wait: 500 }, { cc: 741 }], {}], [1, "substack", 0, "", 12, [], [{ e: 742 }], [{ v: 742 }], [{ waitForThenClick: [".pencraft", "xpath///button[contains(., 'Only Necessary') or contains(., 'Reject')]"] }], [{ cc: 743 }], {}], [1, "summitracing", 1, "", 10, [744], [{ e: 745 }], [{ v: 745 }], [{ h: 744 }], [], {}], [1, "synology", 0, "", 22, [746], [{ e: 746 }], [{ v: 746 }], [{ c: 747 }, { wv: 748 }, { all: true, optional: true, k: 749 }, { c: 750 }], [{ cc: 751 }, { cc: 752 }], {}], [1, "tagconcierge", 2, "", 22, [753], [{ e: 753 }], [{ v: 754 }], [{ if: { v: 755 }, then: [{ k: 755 }], else: [{ c: 756 }, { c: 757 }] }], [{ check: "none", v: 753 }], {}], [1, "takealot.com", 1, "", 22, [758], [{ e: 759 }], [{ e: 759 }], [{ h: 758 }, { if: { e: 760 }, then: [{ eval: "EVAL_TAKEALOT_0" }], else: [] }], [], {}], [1, "tarteaucitron deny", 2, "", 22, [761], [{ e: 761 }], [{ v: 762 }], [{ wait: 500 }, { k: 762 }], [{ eval: "EVAL_TARTEAUCITRON_2" }], {}], [1, "tarteaucitron.js", 2, "", 22, [761], [{ e: 761 }], [{ v: 763 }, { negated: true, e: 762 }], [{ if: { e: 764 }, then: [{ all: true, optional: true, k: 765 }, { k: 763 }], else: [{ k: 766 }, { c: 767 }, { k: 768 }] }], [{ eval: "EVAL_TARTEAUCITRON_2" }], {}], [1, "taunton", 2, "", 22, [769], [{ e: 769 }], [{ e: 770 }], [{ optional: true, all: true, k: 771 }, { k: 772 }], [{ cc: 773 }], {}], [1, "tccCmpAlert", 2, "", 22, [774], [{ e: 774 }], [{ v: 774 }], [{ c: 775 }], [], {}], [1, "Tealium", 2, "", 22, [776], [{ e: 777 }, { eval: "EVAL_TEALIUM_0" }], [{ check: "any", v: 777 }], [{ eval: "EVAL_TEALIUM_1" }, { eval: "EVAL_TEALIUM_DONOTSELL" }, { h: 778 }, { timeout: 1e3, optional: true, c: 779 }], [{ eval: "EVAL_TEALIUM_3" }, { eval: "EVAL_TEALIUM_DONOTSELL_CHECK" }, { check: "none", v: 780 }], {}], [1, "Termly", 2, "", 22, [781], [{ e: 781 }], [{ v: 782 }], [{ if: { e: 783 }, then: [{ k: 783 }], else: [{ c: 784 }, { timeout: 700, w: 785 }, { if: { e: 785 }, then: [{ k: 785 }], else: [{ all: true, c: 786 }, { c: 787 }] }] }], [], {}], [1, "termsfeed", 2, "", 22, [788], [{ e: 788 }], [{ v: 788 }], [{ if: { e: 789 }, then: [{ c: 789 }], else: [{ c: 790 }, { w: 791 }, { all: true, optional: true, k: 792 }, { k: 793 }] }], [], {}], [1, "termsfeed3", 2, "", 22, [794], [{ e: 795 }], [{ v: 795 }], [{ if: { e: 796 }, then: [{ k: 796 }, { wv: 797 }, { c: 797 }], else: [{ h: 794 }] }], [], {}], [1, "Test page CMP", 2, "", 22, [798], [{ e: 799 }], [{ v: 799 }], [{ w: 798 }, { eval: "EVAL_TESTCMP_STEP" }, { k: 798 }], [{ eval: "EVAL_TESTCMP_0" }], {}], [1, "Test page cosmetic CMP", 1, "", 22, [800], [{ e: 801 }], [{ v: 801 }], [{ h: 801 }], [{ wait: 500 }, { eval: "EVAL_TESTCMP_COSMETIC_0" }], {}], [1, "thalia.de", 2, "", 22, [802], [{ e: 803 }], [{ v: 802 }], [{ k: 804 }], [], {}], [1, "thefreedictionary.com", 2, "", 22, [805], [{ e: 805 }], [{ v: 805 }], [{ eval: "EVAL_THEFREEDICTIONARY_0" }], [], {}], [1, "toyota", 0, "", 10, [806], [{ e: 807 }], [{ v: 807 }], [{ c: 808 }], [{ cc: 809 }], {}], [1, "tplink", 0, "", 22, [810], [{ e: 810 }], [{ v: 811 }], [{ all: true, optional: true, k: 812 }, { c: 813 }], [{ cc: 814 }], {}], [1, "trader-joes-com", 1, "", 22, [815], [{ e: 815 }], [{ v: 815 }], [{ h: 815 }], [], {}], [2, "transcend", 1, "", 22, [816], [{ e: 816 }], [{ v: 816 }], [{ setStyle: "display: none !important; z-index: -1 !important; pointer-events: none !important;", selector: "#transcend-consent-manager" }], [], {}], [1, "tropicfeel-com", 2, "", 22, [817], [{ e: 817 }], [{ check: "any", v: 818 }], [{ k: 819 }, { w: 820 }, { all: true, k: 821 }, { k: 822 }], [], {}], [1, "TrustArc-newcm", 2, "", 22, [823], [{ exists: [".truste_popframe.trustarc_newcm_container", "a.required, a.call"] }], [{ visible: [".truste_popframe.trustarc_newcm_container", "a.required, a.call"], check: "any" }], [{ waitForThenClick: [".truste_popframe.trustarc_newcm_container", "a.required"] }], [{ cc: 824 }, { negated: true, cc: 825 }], {}], [1, "truyo", 2, "", 22, [826], [{ e: 827 }], [{ v: 826 }], [{ k: 828 }], [], {}], [1, "trybe", 0, "", 22, [829], [{ e: 829 }], [{ v: 830 }], [{ c: 831 }, { w: 832 }, { optional: true, k: 833 }, { optional: true, k: 834 }, { c: 835 }], [{ wait: 500 }, { cc: 836 }], {}], [1, "tumblr-custom-domain-gdpr-banner", 1, "", 22, [837], [{ e: 837 }], [{ v: 837 }], [{ h: 837 }], [{ check: "none", v: 837 }], {}], [1, "twcc", 0, "", 10, [838], [{ e: 839 }], [{ v: 839 }], [{ c: 840 }], [{ cc: 841 }], {}], [1, "u12-data-protection-notice", 2, "", 22, [842], [{ e: 842 }], [{ v: 843 }], [{ c: 844 }], [], {}], [1, "ubuntu.com", 2, "", 22, [845], [{ any: [{ e: 846 }, { e: 847 }] }], [{ any: [{ v: 848 }, { v: 847 }] }], [{ any: [{ c: 849 }, { c: 850 }] }, { optional: true, all: true, timeout: 500, c: 851 }, { any: [{ c: 852 }, { c: 853 }] }], [{ cc: 854 }], {}], [1, "UK Cookie Consent", 1, "", 22, [855], [{ e: 855 }], [{ e: 856 }], [{ h: 855 }], [{ negated: true, cc: 857 }], {}], [1, "usercentrics-api", 2, "", 22, [], [{ e: 858 }], [{ eval: "EVAL_USERCENTRICS_API_0" }, { if: { e: 859 }, then: [{ timeout: 2e3, wv: 859 }], else: [{ exists: ["#usercentrics-root", "[data-testid=uc-container]"] }, { timeout: 2e3, wv: 860 }] }], [{ if: { exists: ["#usercentrics-root", "[data-testid=uc-deny-all-button]"] }, then: [{ click: ["#usercentrics-root", "[data-testid=uc-deny-all-button]"] }], else: [{ if: { exists: ["#usercentrics-cmp-ui", "[data-action-type=deny]"] }, then: [{ click: ["#usercentrics-cmp-ui", "[data-action-type=deny]"] }], else: [{ eval: "EVAL_USERCENTRICS_API_1" }, { eval: "EVAL_USERCENTRICS_API_2" }] }] }, { removeClass: "overflowHidden", selector: "body", optional: true }], [{ eval: "EVAL_USERCENTRICS_API_6" }], {}], [1, "usercentrics-button", 2, "", 22, [], [{ e: 861 }], [{ v: 862 }], [{ k: 863 }], [{ eval: "EVAL_USERCENTRICS_BUTTON_0" }], {}], [1, "vivenu", 2, "", 22, [], [{ e: 864 }], [{ v: 864 }], [{ c: 865 }], [{ check: "none", v: 864 }], {}], [1, "waitrose.com", 2, "", 22, [866, 867, 868], [{ e: 867 }], [{ v: 867 }], [{ k: 869 }, { wait: 200 }, { eval: "EVAL_WAITROSE_0" }, { k: 870 }], [{ cc: 871 }, { cc: 872 }], {}], [1, "webflow", 2, "", 22, [873], [{ v: 873 }], [{ v: 873 }, { v: 874 }], [{ if: { e: 875 }, then: [{ wait: 500 }, { c: 875 }], else: [{ h: 876 }] }], [{ cc: 877 }], {}], [1, "wiki.gg", 1, "", 22, [878], [{ e: 878 }], [{ v: 878 }], [{ h: 878 }], [], {}], [1, "wix", 0, "", 22, [], [{ e: 879 }], [{ v: 879 }], [{ if: { e: 880 }, then: [{ k: 880 }], else: [{ h: 881 }] }], [], {}], [1, "woo-commerce-com", 2, "", 22, [882], [{ e: 882 }], [{ e: 882 }], [{ k: 883 }, { all: true, c: 50 }, { k: 884 }], [], {}], [1, "workday", 2, "", 22, [885], [{ e: 886 }], [{ v: 885 }], [{ c: 886 }], [{ cc: 887 }], {}], [1, "WP Cookie Notice for GDPR", 2, "", 22, [888], [{ e: 888 }], [{ v: 888 }], [{ c: 889 }], [{ cc: 890 }], {}], [1, "WP DSGVO Tools", 2, "", 22, [891], [{ e: 892 }], [{ check: "any", v: 892 }], [{ c: 893 }], [{ negated: true, cc: 894 }], {}], [1, "wpcc", 1, "", 22, [895], [{ e: 895 }], [{ e: 896 }], [{ h: 895 }], [], {}], [1, "WPConsent", 2, "", 22, [897], [{ e: 898 }], [{ visible: ["#wpconsent-container", "#wpconsent-banner-holder.wpconsent-banner-visible"] }], [{ waitForThenClick: ["#wpconsent-container", "#wpconsent-cancel-all"] }], [{ cc: 899 }], {}], [1, "xgroovy", 1, "", 22, [900], [{ e: 901 }], [{ v: 901 }], [{ h: 900 }], [], {}], [1, "xing.com", 2, "", 22, [], [{ e: 902 }], [{ e: 902 }], [{ k: 903 }, { k: 904 }], [{ cc: 905 }], {}], [1, "xnxx-com", 1, "", 22, [906], [{ any: [{ e: 906 }, { e: 907 }] }], [{ any: [{ v: 906 }, { v: 908 }] }], [{ if: { v: 908 }, then: [{ c: 909 }], else: [{ h: 906 }] }], [], {}], [1, "youtube-desktop", 2, "", 22, [910, 911], [{ e: 912 }, { e: 913 }], [{ v: 912 }], [{ c: 914 }, { wait: 500 }], [{ wait: 500 }, { cc: 439 }], {}], [1, "youtube-mobile", 2, "", 22, [915], [{ e: 916 }], [{ v: 916 }], [{ wait: 500 }, { c: 917 }, { wait: 500 }], [{ wait: 500 }, { cc: 439 }], {}], [1, "zdf", 2, "", 22, [918], [{ e: 918 }], [{ v: 919 }], [{ c: 920 }], [], {}], [1, "cookiealert", 2, "", 11, [], [{ e: 921 }], [{ v: 922 }], [{ k: 923 }, { all: true, optional: true, k: 924 }, { k: 925 }, { eval: "EVAL_COOKIEALERT_0" }], [{ eval: "EVAL_COOKIEALERT_2" }], { intermediate: false }], [1, "piano.io", 2, "", 1, [], [{ e: 926 }], [{ v: 926 }], [{ c: 927 }], [], {}], [1, "auto_AU_help.dropbox.com_4ad_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 928 }], [{ v: 928 }], [{ wait: 500 }, { c: 928 }], [{ timeout: 1e3, check: "none", wv: 928 }], {}], [1, "auto_CA_coasthotels.com_f8m", 0, "^https?://(www\\.)?coasthotels\\.com/", 1, [], [{ e: 929 }], [{ v: 929 }], [{ c: 929 }], [], {}], [1, "auto_CA_epihunter.eu_hd4", 0, "^https?://(www\\.)?combell\\.com/", 1, [], [{ e: 930 }], [{ v: 930 }], [{ wait: 500 }, { c: 930 }], [{ timeout: 1e3, check: "none", wv: 930 }], {}], [1, "auto_CA_nationalarchives.gov.uk_rx0", 0, "^https?://(www\\.)?webarchive\\.nationalarchives\\.gov\\.uk/", 1, [], [{ e: 931 }], [{ v: 931 }], [{ wait: 500 }, { c: 931 }], [{ timeout: 1e3, check: "none", wv: 931 }], {}], [1, "auto_CA_natureconservancy.ca_3pp", 0, "^https?://(www\\.)?natureconservancy\\.ca/", 1, [], [{ e: 932 }], [{ v: 932 }], [{ wait: 500 }, { c: 932 }], [{ timeout: 1e3, check: "none", wv: 932 }], {}], [1, "auto_CA_ontariospca.ca_k32", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 933 }], [{ v: 933 }], [{ wait: 500 }, { c: 933 }], [{ timeout: 1e3, check: "none", wv: 933 }], {}], [1, "auto_CA_phoenixnap.com_hrb", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 934 }], [{ v: 934 }], [{ wait: 500 }, { c: 934 }], [{ timeout: 1e3, check: "none", wv: 934 }], {}], [1, "auto_CH_phoenixnap.com_lx5", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 935 }], [{ v: 935 }], [{ wait: 500 }, { c: 935 }], [{ timeout: 1e3, check: "none", wv: 935 }], {}], [1, "auto_CH_swisscare.com_arx", 0, "^https?://(www\\.)?swisscare\\.com/", 1, [], [{ e: 936 }], [{ v: 936 }], [{ wait: 500 }, { c: 936 }], [{ timeout: 1e3, check: "none", wv: 936 }], {}], [1, "auto_DE_alfaromeo.de_gvg_+2", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 937 }], [{ v: 937 }], [{ wait: 500 }, { c: 937 }], [{ timeout: 1e3, check: "none", wv: 937 }], {}], [1, "auto_DE_huss-licht-ton.de_jj0", 0, "^https?://(www\\.)?huss-licht-ton\\.de/", 1, [], [{ e: 938 }], [{ v: 938 }], [{ wait: 500 }, { c: 938 }], [{ timeout: 1e3, check: "none", wv: 938 }], {}], [1, "auto_DE_modellbau-berlinski.de_8vm", 0, "^https?://(www\\.)?modellbau-berlinski\\.de/", 1, [], [{ e: 939 }], [{ v: 939 }], [{ wait: 500 }, { c: 939 }], [{ timeout: 1e3, check: "none", wv: 939 }], {}], [1, "auto_DE_phoenixnap.com_xq7", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 940 }], [{ v: 940 }], [{ wait: 500 }, { c: 940 }], [{ timeout: 1e3, check: "none", wv: 940 }], {}], [1, "auto_FR_fiat.fr_3fh", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 941 }], [{ v: 941 }], [{ wait: 500 }, { c: 941 }], [{ timeout: 1e3, check: "none", wv: 941 }], {}], [1, "auto_FR_stellantis.com_67q", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 941 }], [{ v: 941 }], [{ wait: 500 }, { c: 941 }], [{ timeout: 1e3, check: "none", wv: 941 }], {}], [1, "auto_GB_dropbox.com_0_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 942 }], [{ v: 942 }], [{ c: 942 }], [], {}], [1, "auto_GB_investing.thisismoney.co.uk_0", 0, "^https?://(www\\.)?thisismoney\\.co\\.uk/", 1, [], [{ e: 943 }], [{ v: 943 }], [{ c: 943 }], [], {}], [1, "auto_GB_village-hotels.co.uk_hsa", 0, "^https?://(www\\.)?village-hotels\\.co\\.uk/", 1, [], [{ e: 944 }], [{ v: 944 }], [{ wait: 500 }, { c: 944 }], [{ timeout: 1e3, check: "none", wv: 944 }], {}], [1, "auto_NL_fiat.nl_trv_+1", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 941 }], [{ v: 941 }], [{ wait: 500 }, { c: 941 }], [{ timeout: 1e3, check: "none", wv: 941 }], {}], [1, "auto_NL_flitsmeister.nl_ws8", 0, "^https?://(www\\.)?cookies\\.flitsmeister\\.com/", 1, [], [{ e: 945 }], [{ v: 945 }], [{ wait: 500 }, { c: 945 }], [{ timeout: 1e3, check: "none", wv: 945 }], {}], [1, "auto_NL_interpolis.nl_jk9", 0, "^https?://(www\\.)?interpolis\\.nl/", 1, [], [{ e: 946 }], [{ v: 946 }], [{ wait: 500 }, { c: 946 }], [{ timeout: 1e3, check: "none", wv: 946 }], {}], [1, "auto_US_dropbox.com_0_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 947 }], [{ v: 947 }], [{ text: "Decline", c: 947 }], [], {}], [1, "coinbase", 2, "^https://(www|help)\\.coinbase\\.com", 11, [], [{ e: 948 }], [{ v: 948 }], [{ k: 949 }, { all: true, optional: true, k: 950 }, { k: 951 }], [{ eval: "EVAL_COINBASE_0" }], { intermediate: false }], [1, "gmx-permission", 2, "^https://plus\\.gmx\\.com/lt(?:[?#]|$)", 1, [952], [{ e: 952 }], [{ v: 953 }], [{ if: { e: 954 }, then: [{ c: 954 }], else: [{ if: { e: 955 }, then: [{ c: 955 }, { wv: 956 }, { c: 956 }], else: [{ c: 956 }] }] }], [{ cc: 957 }], {}], [1, "privacymanager.io", 2, "^https://cmp-consent-tool\\.privacymanager\\.io/", 1, [958, 959], [{ e: 960 }], [{ v: 960 }], [{ if: { e: 961 }, then: [{ k: 961 }, { c: 962 }], else: [{ c: 963 }, { w: 964 }, { w: 965 }, { all: true, optional: true, k: 966 }, { k: 965 }] }], [], {}], [1, "tumblr-com", 2, "^https://(www\\.)?tumblr\\.com/", 21, [967], [{ e: 967 }], [{ check: "any", v: 967 }], [{ waitForThenClick: ["#cmp-app-container iframe", ".cmp-components-button.is-secondary"], timeout: 5e3 }], [], {}], [1, "web.de", 2, "^https://([a-z]*\\.)?web\\.de/|^https://([a-z]*\\.)?gmx\\.net/", 1, [], [{ e: 968 }, { e: 969 }], [{ v: 969 }], [{ c: 969 }], [], {}], [1, "aa-cookie-banner", 2, "^https://(www\\.)?(aa|envoyair|psaairlines)\\.com/", 22, [970], [{ exists: ["adc-cookie-banner", "#cookie-banner"] }], [{ visible: ["adc-cookie-banner", "#cookie-banner"] }], [{ waitForThenClick: ["adc-cookie-banner", "#toast-dismiss-button"] }], [{ cc: 971 }], {}], [1, "abc", 2, "^https://([a-z0-9-]+\\.)?abc\\.net\\.au/", 22, [], [{ e: 972 }], [{ v: 973 }], [{ c: 974 }], [{ cc: 975 }], {}], [1, "abetterrouteplanner", 2, "^https?://(www\\.)?abetterrouteplanner\\.com/", 22, [], [{ e: 976 }], [{ v: 976 }], [{ c: 976 }], [{ negated: true, e: 976 }], {}], [1, "activobank.pt", 2, "^https://(www\\.)?activobank\\.pt", 22, [977], [{ e: 978 }], [{ v: 979 }], [{ c: 980 }], [], {}], [1, "adultfriendfinder", 2, "^https://(www\\.)?adultfriendfinder\\.com/", 22, [981], [{ e: 982 }], [{ v: 982 }], [{ c: 983 }], [{ eval: "EVAL_ADULTFRIENDFINDER_TEST" }], {}], [1, "ah.nl", 0, "^https?://(www\\.)?ah\\.nl/", 10, [], [{ e: 984 }], [{ v: 984 }], [{ c: 985 }], [], {}], [1, "alaskaair", 1, "^https://(www\\.)?alaskaair\\.com/", 22, [986], [{ e: 986 }], [{ v: 986 }], [{ h: 986 }], [], {}], [1, "aliexpress", 2, "^https://([a-z]*\\.)?aliexpress\\.com/", 22, [987], [{ e: 988 }], [{ check: "any", v: 988 }], [{ if: { e: 989 }, then: [{ c: 990 }], else: [{ if: { e: 991 }, then: [{ c: 992 }], else: [{ c: 993 }, { timeout: 5e3, wv: 994 }, { c: 995 }] }] }], [], {}], [1, "ally", 1, "^https://(www\\.)?ally\\.com/", 22, [996], [{ e: 997 }], [{ v: 997 }], [{ h: 996 }], [], {}], [1, "amazon-pay", 2, "^https?://pay\\.amazon\\.", 22, [998], [{ e: 999 }], [{ v: 998 }], [{ k: 999 }], [{ cc: 1e3 }], {}], [1, "app.discuss.io", 2, "^https?://app\\.discuss\\.io/", 10, [], [{ e: 669 }], [{ v: 669 }], [{ c: 669 }], [{ cc: 97 }], {}], [1, "athlinks-com", 1, "^https://(www\\.)?athlinks\\.com/", 22, [1001], [{ e: 1001 }], [{ v: 1002 }], [{ h: 1001 }], [], {}], [1, "auto_AU_acetool.com_n0w_+1", 0, "^https?://(www\\.)?acetool\\.com/|^https?://(www\\.)?unclewiener\\.com/", 10, [], [{ e: 1003 }], [{ v: 1003 }], [{ wait: 500 }, { c: 1003 }], [{ timeout: 1e3, check: "none", wv: 1003 }], {}], [1, "auto_AU_aelfie.com_chb", 0, "^https?://(www\\.)?aelfie\\.com/", 10, [], [{ e: 1004 }], [{ v: 1004 }], [{ wait: 500 }, { c: 1004 }], [{ timeout: 1e3, check: "none", wv: 1004 }], {}], [1, "auto_AU_cam.start.canon_3x5", 0, "^https?://(www\\.)?cam\\.start\\.canon/", 10, [], [{ e: 1005 }], [{ v: 1005 }], [{ wait: 500 }, { c: 1005 }], [{ timeout: 1e3, check: "none", wv: 1005 }], {}], [1, "auto_AU_cam.start.canon_z06", 0, "^https?://(www\\.)?cam\\.start\\.canon/", 10, [], [{ e: 1006 }], [{ v: 1006 }], [{ wait: 500 }, { c: 1006 }], [{ timeout: 1e3, check: "none", wv: 1006 }], {}], [1, "auto_AU_capitoltrades.com_ak9", 0, "^https?://(www\\.)?capitoltrades\\.com/", 10, [], [{ e: 1007 }], [{ v: 1007 }], [{ wait: 500 }, { c: 1007 }], [{ timeout: 1e3, check: "none", wv: 1007 }], {}], [1, "auto_AU_community.dyson.com_lek", 0, "^https?://(www\\.)?community\\.dyson\\.com/", 10, [], [{ e: 1008 }], [{ v: 1008 }], [{ wait: 500 }, { c: 1008 }], [{ timeout: 1e3, check: "none", wv: 1008 }], {}], [1, "auto_AU_conference-board.org_dce", 0, "^https?://(www\\.)?conference-board\\.org/", 10, [], [{ e: 1009 }], [{ v: 1009 }], [{ wait: 500 }, { c: 1009 }], [{ timeout: 1e3, check: "none", wv: 1009 }], {}], [1, "auto_AU_cutterbuck.com_ko7", 0, "^https?://(www\\.)?cutterbuck\\.com/", 10, [], [{ e: 1010 }], [{ v: 1010 }], [{ wait: 500 }, { c: 1010 }], [{ timeout: 1e3, check: "none", wv: 1010 }], {}], [1, "auto_AU_deezer.com_tmf", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 1011 }], [{ v: 1011 }], [{ wait: 500 }, { c: 1011 }], [{ timeout: 1e3, check: "none", wv: 1011 }], {}], [1, "auto_AU_docs.portainer.io_hn5", 0, "^https?://(www\\.)?docs\\.portainer\\.io/", 10, [], [{ e: 1012 }], [{ v: 1012 }], [{ wait: 500 }, { c: 1012 }], [{ timeout: 1e3, check: "none", wv: 1012 }], {}], [1, "auto_AU_flinders.edu.au_7en_+1", 0, "^https?://(www\\.)?flinders\\.edu\\.au/|^https?://(www\\.)?students\\.flinders\\.edu\\.au/", 10, [], [{ e: 1013 }], [{ v: 1013 }], [{ c: 1013 }], [], {}], [1, "auto_AU_flysaa.com_qsm", 0, "^https?://(www\\.)?flysaa\\.com/", 10, [1014], [{ e: 1014 }], [{ v: 1015 }], [{ c: 1016 }], [{ timeout: 1e3, check: "none", wv: 1015 }], {}], [1, "auto_AU_flysas.com_r1s", 0, "^https?://(www\\.)?flysas\\.com/", 10, [], [{ e: 1017 }], [{ v: 1017 }], [{ wait: 500 }, { c: 1017 }], [{ timeout: 1e3, check: "none", wv: 1017 }], {}], [1, "auto_AU_gayseniordating.com_bdr", 0, "^https?://(www\\.)?gayseniordating\\.com/", 10, [], [{ e: 1018 }], [{ v: 1018 }], [{ wait: 500 }, { c: 1018 }], [{ timeout: 1e3, check: "none", wv: 1018 }], {}], [1, "auto_AU_goodmoodprints.com_9fy", 1, "^https?://(www\\.)?goodmoodprints\\.com/", 10, [1019], [{ e: 1019 }], [{ v: 1019 }], [{ h: 1019 }], [{ timeout: 1e3, check: "none", wv: 1019 }], {}], [1, "auto_AU_lush.com_bdf", 0, "^https?://(www\\.)?lush\\.com/", 10, [], [{ e: 1020 }], [{ v: 1020 }], [{ wait: 500 }, { c: 1020 }], [{ timeout: 1e3, check: "none", wv: 1020 }], {}], [1, "auto_AU_newyork.doverstreetmarket.com_j5q", 0, "^https?://(www\\.)?newyork\\.doverstreetmarket\\.com/", 10, [], [{ e: 1021 }], [{ v: 1021 }], [{ wait: 500 }, { c: 1021 }], [{ timeout: 1e3, check: "none", wv: 1021 }], {}], [1, "auto_AU_newyork.doverstreetmarket.com_k2q", 0, "^https?://(www\\.)?newyork\\.doverstreetmarket\\.com/", 10, [], [{ e: 1022 }], [{ v: 1022 }], [{ wait: 500 }, { c: 1022 }], [{ timeout: 1e3, check: "none", wv: 1022 }], {}], [1, "auto_AU_niwaki.com_w41", 0, "^https?://(www\\.)?niwaki\\.com/", 10, [], [{ e: 1023 }], [{ v: 1023 }], [{ wait: 500 }, { c: 1023 }], [{ timeout: 1e3, check: "none", wv: 1023 }], {}], [1, "auto_AU_pichunter.com_zhf", 0, "^https?://(www\\.)?pichunter\\.com/", 10, [], [{ e: 1024 }], [{ v: 1024 }], [{ wait: 500 }, { c: 1024 }], [{ timeout: 1e3, check: "none", wv: 1024 }], {}], [1, "auto_AU_pnp.co.za_9lg", 0, "^https?://(www\\.)?pnp\\.co\\.za/", 10, [], [{ e: 1025 }], [{ v: 1025 }], [{ wait: 500 }, { c: 1025 }], [{ timeout: 1e3, check: "none", wv: 1025 }], {}], [1, "auto_AU_publicdomainreview.org_7fg", 0, "^https?://(www\\.)?publicdomainreview\\.org/", 10, [], [{ e: 1026 }], [{ v: 1026 }], [{ wait: 500 }, { c: 1026 }], [{ timeout: 1e3, check: "none", wv: 1026 }], {}], [1, "auto_AU_qldnaturistassoc.org_k53", 0, "^https?://(www\\.)?qldnaturistassoc\\.org/", 10, [], [{ e: 1027 }], [{ v: 1027 }], [{ wait: 500 }, { c: 1027 }], [{ timeout: 1e3, check: "none", wv: 1027 }], {}], [1, "auto_AU_rki.de_tp5", 0, "^https?://(www\\.)?rki\\.de/", 10, [], [{ e: 1028 }], [{ v: 1028 }], [{ wait: 500 }, { c: 1028 }], [{ timeout: 1e3, check: "none", wv: 1028 }], {}], [1, "auto_AU_solesavy.com_8q6", 0, "^https?://(www\\.)?solesavy\\.com/", 10, [], [{ e: 1029 }], [{ v: 1029 }], [{ wait: 500 }, { c: 1029 }], [{ timeout: 1e3, check: "none", wv: 1029 }], {}], [1, "auto_AU_staff.flinders.edu.au_cpu", 0, "^https?://(www\\.)?staff\\.flinders\\.edu\\.au/", 10, [], [{ e: 1013 }], [{ v: 1013 }], [{ wait: 500 }, { c: 1013 }], [{ timeout: 1e3, check: "none", wv: 1013 }], {}], [1, "auto_AU_telekom.hu_4lu", 0, "^https?://(www\\.)?telekom\\.hu/", 10, [], [{ e: 1030 }], [{ v: 1030 }], [{ wait: 500 }, { c: 1030 }], [{ timeout: 1e3, check: "none", wv: 1030 }], {}], [1, "auto_AU_tourismnewbrunswick.ca_bjd", 0, "^https?://(www\\.)?tourismnewbrunswick\\.ca/", 10, [], [{ e: 1031 }], [{ v: 1031 }], [{ wait: 500 }, { c: 1031 }], [{ timeout: 1e3, check: "none", wv: 1031 }], {}], [1, "auto_AU_tripcentral.ca_v7v", 0, "^https?://(www\\.)?tripcentral\\.ca/", 10, [], [{ e: 1032 }], [{ v: 1032 }], [{ wait: 500 }, { c: 1032 }], [{ timeout: 1e3, check: "none", wv: 1032 }], {}], [1, "auto_AU_u-buy.com.au_rg0", 0, "^https?://(www\\.)?u-buy\\.com\\.au/", 10, [], [{ e: 1033 }], [{ v: 1033 }], [{ wait: 500 }, { c: 1033 }], [{ timeout: 1e3, check: "none", wv: 1033 }], {}], [1, "auto_AU_whitepages.co.com_4cs", 0, "^https?://(www\\.)?whitepages\\.co\\.com/", 10, [], [{ e: 1034 }], [{ v: 1034 }], [{ wait: 500 }, { c: 1034 }], [{ timeout: 1e3, check: "none", wv: 1034 }], {}], [1, "auto_CA_407etr.com_wrd", 0, "^https?://(www\\.)?407etr\\.com/", 10, [], [{ e: 1035 }], [{ v: 1035 }], [{ wait: 500 }, { c: 1035 }], [{ timeout: 1e3, check: "none", wv: 1035 }], {}], [1, "auto_CA_arte.tv_7nv", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 1036 }], [{ v: 1036 }], [{ c: 1036 }], [], {}], [1, "auto_CA_babel.hathitrust.org_i6g", 0, "^https?://(www\\.)?babel\\.hathitrust\\.org/", 10, [], [{ e: 1037 }], [{ v: 1037 }], [{ wait: 500 }, { c: 1037 }], [{ timeout: 1e3, check: "none", wv: 1037 }], {}], [1, "auto_CA_belairdirect.com_wic", 0, "^https?://(www\\.)?belairdirect\\.com/", 10, [], [{ e: 1038 }], [{ v: 1038 }], [{ wait: 500 }, { c: 1038 }], [{ timeout: 1e3, check: "none", wv: 1038 }], {}], [1, "auto_CA_bet365.bet.br_kkx", 0, "^https?://(www\\.)?bet365\\.bet\\.br/", 10, [], [{ e: 1039 }], [{ v: 1039 }], [{ wait: 500 }, { c: 1039 }], [{ timeout: 1e3, check: "none", wv: 1039 }], {}], [1, "auto_CA_blackdiamondequipment.com_dwp_+8", 0, "^https?://(www\\.)?blackdiamondequipment\\.com/|^https?://(www\\.)?facetofacegames\\.com/|^https?://(www\\.)?suzyshier\\.com/|^https?://(www\\.)?urban-planet\\.com/|^https?://(www\\.)?eu\\.blackdiamondequipment\\.com/|^https?://(www\\.)?bushwear\\.co\\.uk/|^https?://(www\\.)?directdoors\\.com/|^https?://(www\\.)?keenfootwear\\.com/|^https?://(www\\.)?shop\\.panasonic\\.com/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_CA_brazzers.com_kik", 0, "^https?://(www\\.)?brazzers\\.com/", 10, [], [{ e: 1041 }], [{ v: 1041 }], [{ wait: 500 }, { c: 1041 }], [{ timeout: 1e3, check: "none", wv: 1041 }], {}], [1, "auto_CA_busbud.com_0sh", 0, "^https?://(www\\.)?busbud\\.com/", 10, [], [{ e: 1042 }], [{ v: 1042 }], [{ wait: 500 }, { c: 1042 }], [{ timeout: 1e3, check: "none", wv: 1042 }], {}], [1, "auto_CA_cegeplimoilou.ca_9tg", 0, "^https?://(www\\.)?cegeplimoilou\\.ca/", 10, [], [{ e: 1043 }], [{ v: 1043 }], [{ wait: 500 }, { c: 1043 }], [{ timeout: 1e3, check: "none", wv: 1043 }], {}], [1, "auto_CA_cfo.coop_edx", 0, "^https?://(www\\.)?cfo\\.coop/", 10, [], [{ e: 1044 }], [{ v: 1044 }], [{ wait: 500 }, { c: 1044 }], [{ timeout: 1e3, check: "none", wv: 1044 }], {}], [1, "auto_CA_chipolo.net_zo9", 0, "^https?://(www\\.)?chipolo\\.net/", 10, [], [{ e: 1045 }], [{ v: 1045 }], [{ wait: 500 }, { c: 1045 }], [{ timeout: 1e3, check: "none", wv: 1045 }], {}], [1, "auto_CA_cmpa-acpm.ca_nja", 0, "^https?://(www\\.)?cmpa-acpm\\.ca/", 10, [], [{ e: 1046 }], [{ v: 1046 }], [{ c: 1046 }], [], {}], [1, "auto_CA_denniskirk.com_ezr", 0, "^https?://(www\\.)?denniskirk\\.com/", 10, [], [{ e: 1047 }], [{ v: 1047 }], [{ wait: 500 }, { c: 1047 }], [{ timeout: 1e3, check: "none", wv: 1047 }], {}], [1, "auto_CA_doyondespres.com_4jr", 0, "^https?://(www\\.)?doyondespres\\.com/", 10, [], [{ e: 1048 }], [{ v: 1048 }], [{ wait: 500 }, { c: 1048 }], [{ timeout: 1e3, check: "none", wv: 1048 }], {}], [1, "auto_CA_f6s.com_yjw", 0, "^https?://(www\\.)?f6s\\.com/", 10, [], [{ e: 1049 }], [{ v: 1049 }], [{ wait: 500 }, { c: 1049 }], [{ timeout: 1e3, check: "none", wv: 1049 }], {}], [1, "auto_CA_fischersports.com_1yr", 0, "^https?://(www\\.)?fischersports\\.com/", 10, [], [{ e: 1050 }], [{ v: 1050 }], [{ wait: 500 }, { c: 1050 }], [{ timeout: 1e3, check: "none", wv: 1050 }], {}], [1, "auto_CA_golfavenue.ca_ero_+1", 0, "^https?://(www\\.)?golfavenue\\.ca/|^https?://(www\\.)?golfbidder\\.co\\.uk/", 10, [], [{ e: 1051 }], [{ v: 1051 }], [{ wait: 500 }, { c: 1051 }], [{ timeout: 1e3, check: "none", wv: 1051 }], {}], [1, "auto_CA_golftown.com_zcn", 0, "^https?://(www\\.)?golftown\\.com/", 10, [], [{ e: 1052 }], [{ v: 1052 }], [{ wait: 500 }, { c: 1052 }], [{ timeout: 1e3, check: "none", wv: 1052 }], {}], [1, "auto_CA_intact.ca_gax", 0, "^https?://(www\\.)?intact\\.ca/", 10, [], [{ e: 1053 }], [{ v: 1053 }], [{ wait: 500 }, { c: 1053 }], [{ timeout: 1e3, check: "none", wv: 1053 }], {}], [1, "auto_CA_lethpolytech.ca_1r1", 0, "^https?://(www\\.)?lethpolytech\\.ca/", 10, [], [{ e: 1054 }], [{ v: 1054 }], [{ wait: 500 }, { c: 1054 }], [{ timeout: 1e3, check: "none", wv: 1054 }], {}], [1, "auto_CA_libgen.help_7s8", 0, "^https?://(www\\.)?libgen\\.help/", 10, [], [{ e: 1055 }], [{ v: 1055 }], [{ wait: 500 }, { c: 1055 }], [{ timeout: 1e3, check: "none", wv: 1055 }], {}], [1, "auto_CA_nationalarchives.gov.uk_vgx", 0, "^https?://(www\\.)?webarchive\\.nationalarchives\\.gov\\.uk/", 10, [], [{ e: 1056 }], [{ v: 1056 }], [{ wait: 500 }, { c: 1056 }], [{ timeout: 1e3, check: "none", wv: 1056 }], {}], [1, "auto_CA_papajohns.ca_n5j", 0, "^https?://(www\\.)?papajohns\\.ca/", 10, [], [{ e: 1057 }], [{ v: 1057 }], [{ wait: 500 }, { c: 1057 }], [{ timeout: 1e3, check: "none", wv: 1057 }], {}], [1, "auto_CA_pcsupport.lenovo.com_dxi", 0, "^https?://(www\\.)?pcsupport\\.lenovo\\.com/", 10, [], [{ e: 1058 }], [{ v: 1058 }], [{ wait: 500 }, { c: 1058 }], [{ timeout: 1e3, check: "none", wv: 1058 }], {}], [1, "auto_CA_plex.tv_ee4_+1", 0, "^https?://(www\\.)?plex\\.tv/|^https?://(www\\.)?support\\.plex\\.tv/", 10, [], [{ e: 1059 }], [{ v: 1059 }], [{ wait: 500 }, { c: 1059 }], [{ timeout: 1e3, check: "none", wv: 1059 }], {}], [1, "auto_CA_promessedefleurs.com_noy_+1", 0, "^https?://(www\\.)?promessedefleurs\\.com/|^https?://(www\\.)?meillandrichardier\\.com/", 10, [], [{ e: 1060 }], [{ v: 1060 }], [{ wait: 500 }, { c: 1060 }], [{ timeout: 1e3, check: "none", wv: 1060 }], {}], [1, "auto_CA_queerty.com_fb8_+1", 0, "^https?://(www\\.)?queerty\\.com/|^https?://(www\\.)?pestor\\.nl/", 10, [], [{ e: 1061 }], [{ v: 1061 }], [{ wait: 500 }, { c: 1061 }], [{ timeout: 1e3, check: "none", wv: 1061 }], {}], [1, "auto_CA_remitly.com_mm4", 0, "^https?://(www\\.)?remitly\\.com/", 10, [], [{ e: 1062 }], [{ v: 1062 }], [{ wait: 500 }, { c: 1062 }], [{ timeout: 1e3, check: "none", wv: 1062 }], {}], [1, "auto_CA_reviewed.com_w2k", 0, "^https?://(www\\.)?reviewed\\.com/", 10, [], [{ e: 1063 }], [{ v: 1063 }], [{ wait: 500 }, { c: 1063 }], [{ timeout: 1e3, check: "none", wv: 1063 }], {}], [1, "auto_CA_sheetmusicplus.com_b7d", 0, "^https?://(www\\.)?sheetmusicplus\\.com/", 10, [], [{ e: 1064 }], [{ v: 1064 }], [{ wait: 500 }, { c: 1064 }], [{ timeout: 1e3, check: "none", wv: 1064 }], {}], [1, "auto_CA_skipthedishes.com_1sg", 0, "^https?://(www\\.)?skipthedishes\\.com/", 10, [], [{ e: 1065 }], [{ v: 1065 }], [{ c: 1065 }], [], {}], [1, "auto_CA_streamingthe.net_v8e", 0, "^https?://(www\\.)?streamingthe\\.net/", 10, [], [{ e: 1066 }], [{ v: 1066 }], [{ wait: 500 }, { c: 1066 }], [{ timeout: 1e3, check: "none", wv: 1066 }], {}], [1, "auto_CA_ticketsource.com_kiv", 0, "^https?://(www\\.)?ticketsource\\.com/", 10, [], [{ e: 1067 }], [{ v: 1067 }], [{ wait: 500 }, { c: 1067 }], [{ timeout: 1e3, check: "none", wv: 1067 }], {}], [1, "auto_CA_ubereats.com_4nx", 0, "^https?://(www\\.)?ubereats\\.com/", 10, [], [{ e: 1068 }], [{ v: 1068 }], [{ wait: 500 }, { c: 1068 }], [{ timeout: 1e3, check: "none", wv: 1068 }], {}], [1, "auto_CA_wealthsimple.com_kgz", 0, "^https?://(www\\.)?wealthsimple\\.com/", 10, [], [{ e: 1069 }], [{ v: 1069 }], [{ wait: 500 }, { c: 1069 }], [{ timeout: 1e3, check: "none", wv: 1069 }], {}], [1, "auto_CA_xplore.ca_wwx", 0, "^https?://(www\\.)?xplore\\.ca/", 10, [], [{ e: 1070 }], [{ v: 1070 }], [{ wait: 500 }, { c: 1070 }], [{ timeout: 1e3, check: "none", wv: 1070 }], {}], [1, "auto_CH_3djake.ch_6k2_+3", 0, "^https?://(www\\.)?3djake\\.ch/|^https?://(www\\.)?ecco-verde\\.ch/|^https?://(www\\.)?vitalabo\\.ch/|^https?://(www\\.)?3djake\\.de/", 10, [], [{ e: 1071 }], [{ v: 1071 }], [{ wait: 500 }, { c: 1071 }], [{ timeout: 1e3, check: "none", wv: 1071 }], {}], [1, "auto_CH_airbnb.de_83z", 0, "^https?://(www\\.)?airbnb\\.de/", 10, [], [{ e: 1072 }], [{ v: 1072 }], [{ wait: 500 }, { c: 1072 }], [{ timeout: 1e3, check: "none", wv: 1072 }], {}], [1, "auto_CH_alpenvereinaktiv.com_ms8_+1", 0, "^https?://(www\\.)?alpenvereinaktiv\\.com/|^https?://(www\\.)?maps\\.viamala\\.ch/", 10, [], [{ e: 1073 }], [{ v: 1073 }], [{ wait: 500 }, { c: 1073 }], [{ timeout: 1e3, check: "none", wv: 1073 }], {}], [1, "auto_CH_android.bestsecret.com_m12_+2", 0, "^https?://(www\\.)?android\\.bestsecret\\.com/|^https?://(www\\.)?bestsecret\\.com/|^https?://(www\\.)?orders\\.bestsecret\\.com/", 10, [], [{ e: 1074 }], [{ v: 1074 }], [{ wait: 500 }, { c: 1074 }], [{ timeout: 1e3, check: "none", wv: 1074 }], {}], [1, "auto_CH_aoc.com_dib", 0, "^https?://(www\\.)?aoc\\.com/", 10, [], [{ e: 1075 }], [{ v: 1075 }], [{ wait: 500 }, { c: 1075 }], [{ timeout: 1e3, check: "none", wv: 1075 }], {}], [1, "auto_CH_arte.tv_ln7", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 1076 }], [{ v: 1076 }], [{ wait: 500 }, { c: 1076 }], [{ timeout: 1e3, check: "none", wv: 1076 }], {}], [1, "auto_CH_arttv.ch_2lm", 0, "^https?://(www\\.)?arttv\\.ch/", 10, [], [{ e: 1077 }], [{ v: 1077 }], [{ wait: 500 }, { c: 1077 }], [{ timeout: 1e3, check: "none", wv: 1077 }], {}], [1, "auto_CH_ascona.il-centro.ch_59k", 0, "^https?://(www\\.)?ascona\\.il-centro\\.ch/", 10, [], [{ e: 1078 }], [{ v: 1078 }], [{ wait: 500 }, { c: 1078 }], [{ timeout: 1e3, check: "none", wv: 1078 }], {}], [1, "auto_CH_astag.ch_1if", 0, "^https?://(www\\.)?astag\\.ch/", 10, [], [{ e: 1079 }], [{ v: 1079 }], [{ wait: 500 }, { c: 1079 }], [{ timeout: 1e3, check: "none", wv: 1079 }], {}], [1, "auto_CH_auto.swiss_z35", 0, "^https?://(www\\.)?auto\\.swiss/", 10, [], [{ e: 1080 }], [{ v: 1080 }], [{ wait: 500 }, { c: 1080 }], [{ timeout: 1e3, check: "none", wv: 1080 }], {}], [1, "auto_CH_autodoc.de_krd", 0, "^https?://(www\\.)?autodoc\\.de/", 10, [], [{ e: 1081 }], [{ v: 1081 }], [{ wait: 500 }, { c: 1081 }], [{ timeout: 1e3, check: "none", wv: 1081 }], {}], [1, "auto_CH_bandlab.com_jul", 0, "^https?://(www\\.)?bandlab\\.com/", 10, [], [{ e: 1082 }], [{ v: 1082 }], [{ wait: 500 }, { c: 1082 }], [{ timeout: 1e3, check: "none", wv: 1082 }], {}], [1, "auto_CH_bitbox.swiss_w0n", 0, "^https?://(www\\.)?bitbox\\.swiss/", 10, [], [{ e: 1083 }], [{ v: 1083 }], [{ wait: 500 }, { c: 1083 }], [{ timeout: 1e3, check: "none", wv: 1083 }], {}], [1, "auto_CH_blogdumoderateur.com_8p6", 0, "^https?://(www\\.)?blogdumoderateur\\.com/", 10, [], [{ e: 1084 }], [{ v: 1084 }], [{ wait: 500 }, { c: 1084 }], [{ timeout: 1e3, check: "none", wv: 1084 }], {}], [1, "auto_CH_blutspendezurich.ch_s0f", 0, "^https?://(www\\.)?blutspendezurich\\.ch/", 10, [], [{ e: 1085 }], [{ v: 1085 }], [{ wait: 500 }, { c: 1085 }], [{ timeout: 1e3, check: "none", wv: 1085 }], {}], [1, "auto_CH_cardmarket.com_15t", 0, "^https?://(www\\.)?cardmarket\\.com/", 10, [], [{ e: 1086 }], [{ v: 1086 }], [{ wait: 500 }, { c: 1086 }], [{ timeout: 1e3, check: "none", wv: 1086 }], {}], [1, "auto_CH_ch.rotho.com_h99_+2", 0, "^https?://(www\\.)?ch\\.rotho\\.com/|^https?://(www\\.)?reclam\\.de/|^https?://(www\\.)?dacianer\\.de/", 10, [], [{ e: 1087 }], [{ v: 1087 }], [{ wait: 500 }, { c: 1087 }], [{ timeout: 1e3, check: "none", wv: 1087 }], {}], [1, "auto_CH_chambres-hotes.fr_hay", 0, "^https?://(www\\.)?chambres-hotes\\.fr/", 10, [], [{ e: 1088 }], [{ v: 1088 }], [{ wait: 500 }, { c: 1088 }], [{ timeout: 1e3, check: "none", wv: 1088 }], {}], [1, "auto_CH_chrono24.ch_dve_+1", 0, "^https?://(www\\.)?chrono24\\.ch/|^https?://(www\\.)?chrono24\\.de/", 10, [], [{ e: 1089 }], [{ v: 1089 }], [{ wait: 500 }, { c: 1089 }], [{ timeout: 1e3, check: "none", wv: 1089 }], {}], [1, "auto_CH_chrono24.com_lj5_+1", 0, "^https?://(www\\.)?chrono24\\.com/|^https?://(www\\.)?chrono24\\.co\\.uk/", 10, [], [{ e: 1089 }], [{ v: 1089 }], [{ wait: 500 }, { c: 1089 }], [{ timeout: 1e3, check: "none", wv: 1089 }], {}], [1, "auto_CH_ersatzteil-shop24.de_mh5_+1", 0, "^https?://(www\\.)?ersatzteil-shop24\\.de/|^https?://(www\\.)?der-rasenmaeher\\.de/", 10, [], [{ e: 1090 }], [{ v: 1090 }], [{ wait: 500 }, { c: 1090 }], [{ timeout: 1e3, check: "none", wv: 1090 }], {}], [1, "auto_CH_euroairport.com_3yl_+4", 0, "^https?://(www\\.)?euroairport\\.com/|^https?://(www\\.)?carnavalet\\.paris\\.fr/|^https?://(www\\.)?ifrap\\.org/|^https?://(www\\.)?mam\\.paris\\.fr/|^https?://(www\\.)?petitpalais\\.paris\\.fr/", 10, [], [{ e: 1091 }], [{ v: 1091 }], [{ wait: 500 }, { c: 1091 }], [{ timeout: 1e3, check: "none", wv: 1091 }], {}], [1, "auto_CH_europarl.europa.eu_y2d", 0, "^https?://(www\\.)?europarl\\.europa\\.eu/", 10, [], [{ e: 1092 }], [{ v: 1092 }], [{ wait: 500 }, { c: 1092 }], [{ timeout: 1e3, check: "none", wv: 1092 }], {}], [1, "auto_CH_feldschloesschen.ch_4mc", 0, "^https?://(www\\.)?feldschloesschen\\.ch/", 10, [], [{ e: 1093 }], [{ v: 1093 }], [{ wait: 500 }, { c: 1093 }], [{ timeout: 1e3, check: "none", wv: 1093 }], {}], [1, "auto_CH_fhgr.ch_9nr", 0, "^https?://(www\\.)?fhgr\\.ch/", 10, [], [{ e: 1094 }], [{ v: 1094 }], [{ wait: 500 }, { c: 1094 }], [{ timeout: 1e3, check: "none", wv: 1094 }], {}], [1, "auto_CH_fondationbeyeler.ch_o13", 0, "^https?://(www\\.)?fondationbeyeler\\.ch/", 10, [], [{ e: 1087 }], [{ v: 1087 }], [{ wait: 500 }, { c: 1087 }], [{ timeout: 1e3, check: "none", wv: 1087 }], {}], [1, "auto_CH_fontis-shop.ch_ksd", 0, "^https?://(www\\.)?fontis-shop\\.ch/", 10, [], [{ e: 1095 }], [{ v: 1095 }], [{ wait: 500 }, { c: 1095 }], [{ timeout: 1e3, check: "none", wv: 1095 }], {}], [1, "auto_CH_frankenspalter.ch_jo0", 0, "^https?://(www\\.)?frankenspalter\\.ch/", 10, [], [{ e: 1096 }], [{ v: 1096 }], [{ wait: 500 }, { c: 1096 }], [{ timeout: 1e3, check: "none", wv: 1096 }], {}], [1, "auto_CH_frankenspalter.ch_l2w", 0, "^https?://(www\\.)?frankenspalter\\.ch/", 10, [], [{ e: 1097 }], [{ v: 1097 }], [{ wait: 500 }, { c: 1097 }], [{ timeout: 1e3, check: "none", wv: 1097 }], {}], [1, "auto_CH_fruugoschweiz.com_6k8", 0, "^https?://(www\\.)?fruugoschweiz\\.com/", 10, [], [{ e: 1098 }], [{ v: 1098 }], [{ wait: 500 }, { c: 1098 }], [{ timeout: 1e3, check: "none", wv: 1098 }], {}], [1, "auto_CH_gabrielweinberg.com_56i", 0, "^https?://(www\\.)?gabrielweinberg\\.com/", 10, [], [{ e: 1099 }], [{ v: 1099 }], [{ wait: 500 }, { c: 1099 }], [{ timeout: 1e3, check: "none", wv: 1099 }], {}], [1, "auto_CH_galaxus.de_sqv_+1", 0, "^https?://(www\\.)?galaxus\\.de/|^https?://(www\\.)?galaxus\\.fr/", 10, [], [{ e: 1100 }], [{ v: 1100 }], [{ wait: 500 }, { c: 1100 }], [{ timeout: 1e3, check: "none", wv: 1100 }], {}], [1, "auto_CH_gstaadmenuhinfestival.ch_vp3", 0, "^https?://(www\\.)?gstaadmenuhinfestival\\.ch/", 10, [], [{ e: 1101 }], [{ v: 1101 }], [{ wait: 500 }, { c: 1101 }], [{ timeout: 1e3, check: "none", wv: 1101 }], {}], [1, "auto_CH_haushaltstipps.com_9kx_+1", 0, "^https?://(www\\.)?haushaltstipps\\.com/|^https?://(www\\.)?alltours\\.de/", 10, [], [{ e: 1087 }], [{ v: 1087 }], [{ wait: 500 }, { c: 1087 }], [{ timeout: 1e3, check: "none", wv: 1087 }], {}], [1, "auto_CH_independentxpress.de_0qj", 0, "^https?://(www\\.)?independentxpress\\.de/", 10, [], [{ e: 1102 }], [{ v: 1102 }], [{ wait: 500 }, { c: 1102 }], [{ timeout: 1e3, check: "none", wv: 1102 }], {}], [1, "auto_CH_instant-gaming.com_8j3", 0, "^https?://(www\\.)?instant-gaming\\.com/", 10, [], [{ e: 1103 }], [{ v: 1103 }], [{ wait: 500 }, { c: 1103 }], [{ timeout: 1e3, check: "none", wv: 1103 }], {}], [1, "auto_CH_kinopoisk.ru_w3g_+14", 0, "^https?://(www\\.)?kinopoisk\\.ru/|^https?://(www\\.)?market\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com/|^https?://(www\\.)?translate\\.yandex\\.ru/|^https?://(www\\.)?yandex\\.by/|^https?://(www\\.)?local\\.yandex\\.com/|^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?360\\.yandex\\.ru/|^https?://(www\\.)?360\\.yandex\\.com/|^https?://(www\\.)?music\\.yandex\\.ru/|^https?://(www\\.)?hd\\.kinopoisk\\.ru/|^https?://(www\\.)?wap\\.yandex\\.com/", 10, [], [{ e: 1104 }], [{ v: 1104 }], [{ wait: 500 }, { c: 1104 }], [{ timeout: 1e3, check: "none", wv: 1104 }], {}], [1, "auto_CH_kobo.com_p4l", 0, "^https?://(www\\.)?kobo\\.com/", 10, [], [{ e: 1105 }], [{ v: 1105 }], [{ wait: 500 }, { c: 1105 }], [{ timeout: 1e3, check: "none", wv: 1105 }], {}], [1, "auto_CH_labanquepostale.fr_o6f_+1", 0, "^https?://(www\\.)?labanquepostale\\.fr/|^https?://(www\\.)?labanquepostale\\.com/", 10, [], [{ e: 1106 }], [{ v: 1106 }], [{ wait: 500 }, { c: 1106 }], [{ timeout: 1e3, check: "none", wv: 1106 }], {}], [1, "auto_CH_laposte.fr_ctu_+2", 0, "^https?://(www\\.)?laposte\\.fr/|^https?://(www\\.)?localiser\\.laposte\\.fr/|^https?://(www\\.)?aide\\.laposte\\.fr/", 10, [], [{ e: 1107 }], [{ v: 1107 }], [{ wait: 500 }, { c: 1107 }], [{ timeout: 1e3, check: "none", wv: 1107 }], {}], [1, "auto_CH_larian.com_kp5", 0, "^https?://(www\\.)?larian\\.com/", 10, [], [{ e: 1108 }], [{ v: 1108 }], [{ wait: 500 }, { c: 1108 }], [{ timeout: 1e3, check: "none", wv: 1108 }], {}], [1, "auto_CH_louis-moto.ch_5yk", 0, "^https?://(www\\.)?louis-moto\\.ch/", 10, [], [{ e: 1109 }], [{ v: 1109 }], [{ wait: 500 }, { c: 1109 }], [{ timeout: 1e3, check: "none", wv: 1109 }], {}], [1, "auto_CH_manufactum.ch_ag0_+1", 0, "^https?://(www\\.)?manufactum\\.ch/|^https?://(www\\.)?manufactum\\.de/", 10, [], [{ e: 1110 }], [{ v: 1110 }], [{ wait: 500 }, { c: 1110 }], [{ timeout: 1e3, check: "none", wv: 1110 }], {}], [1, "auto_CH_maps.engadin.ch_m40_+3", 0, "^https?://(www\\.)?maps\\.engadin\\.ch/|^https?://(www\\.)?outdoor\\.glarnerland\\.ch/|^https?://(www\\.)?altenberg\\.de/|^https?://(www\\.)?tourenplaner-rheinland-pfalz\\.de/", 10, [], [{ e: 1073 }], [{ v: 1073 }], [{ wait: 500 }, { c: 1073 }], [{ timeout: 1e3, check: "none", wv: 1073 }], {}], [1, "auto_CH_mein-kraeuterkeller.de_zjh", 0, "^https?://(www\\.)?mein-kraeuterkeller\\.de/", 10, [], [{ e: 1111 }], [{ v: 1111 }], [{ wait: 500 }, { c: 1111 }], [{ timeout: 1e3, check: "none", wv: 1111 }], {}], [1, "auto_CH_meintiptopf.ch_84l", 0, "^https?://(www\\.)?meintiptopf\\.ch/", 10, [], [{ e: 1112 }], [{ v: 1112 }], [{ wait: 500 }, { c: 1112 }], [{ timeout: 1e3, check: "none", wv: 1112 }], {}], [1, "auto_CH_mio.se_hd0", 0, "^https?://(www\\.)?mio\\.se/", 10, [], [{ e: 1113 }], [{ v: 1113 }], [{ wait: 500 }, { c: 1113 }], [{ timeout: 1e3, check: "none", wv: 1113 }], {}], [1, "auto_CH_moebel24.ch_fck_+1", 0, "^https?://(www\\.)?moebel24\\.ch/|^https?://(www\\.)?moebel\\.de/", 10, [], [{ e: 1114 }], [{ v: 1114 }], [{ wait: 500 }, { c: 1114 }], [{ timeout: 1e3, check: "none", wv: 1114 }], {}], [1, "auto_CH_mrporter.com_0nl_+1", 0, "^https?://(www\\.)?mrporter\\.com/|^https?://(www\\.)?net-a-porter\\.com/", 10, [], [{ e: 1115 }], [{ v: 1115 }], [{ wait: 500 }, { c: 1115 }], [{ timeout: 1e3, check: "none", wv: 1115 }], {}], [1, "auto_CH_platform.openai.com_g5y", 0, "^https?://(www\\.)?platform\\.openai\\.com/", 10, [], [{ e: 1116 }], [{ v: 1116 }], [{ wait: 500 }, { c: 1116 }], [{ timeout: 1e3, check: "none", wv: 1116 }], {}], [1, "auto_CH_pmphotomedia.ch_ume", 0, "^https?://(www\\.)?pmphotomedia\\.ch/", 10, [], [{ e: 1117 }], [{ v: 1117 }], [{ wait: 500 }, { c: 1117 }], [{ timeout: 1e3, check: "none", wv: 1117 }], {}], [1, "auto_CH_radio1.ch_1v2", 0, "^https?://(www\\.)?radio1\\.ch/", 10, [], [{ e: 1118 }], [{ v: 1118 }], [{ wait: 500 }, { c: 1118 }], [{ timeout: 1e3, check: "none", wv: 1118 }], {}], [1, "auto_CH_renens.ch_2uc", 0, "^https?://(www\\.)?renens\\.ch/", 10, [], [{ e: 1119 }], [{ v: 1119 }], [{ wait: 500 }, { c: 1119 }], [{ timeout: 1e3, check: "none", wv: 1119 }], {}], [1, "auto_CH_sac-uto.ch_3gu", 0, "^https?://(www\\.)?sac-uto\\.ch/", 10, [], [{ e: 1120 }], [{ v: 1120 }], [{ wait: 500 }, { c: 1120 }], [{ timeout: 1e3, check: "none", wv: 1120 }], {}], [1, "auto_CH_seetickets.com_4b2", 0, "^https?://(www\\.)?seetickets\\.com/", 10, [], [{ e: 1121 }], [{ v: 1121 }], [{ wait: 500 }, { c: 1121 }], [{ timeout: 1e3, check: "none", wv: 1121 }], {}], [1, "auto_CH_sparkasse.de_lwr", 0, "^https?://(www\\.)?sparkasse\\.de/", 10, [], [{ e: 1122 }], [{ v: 1122 }], [{ wait: 500 }, { c: 1122 }], [{ timeout: 1e3, check: "none", wv: 1122 }], {}], [1, "auto_CH_sva-bl.ch_h2k_+1", 0, "^https?://(www\\.)?sva-bl\\.ch/|^https?://(www\\.)?uni-bremen\\.de/", 10, [], [{ e: 1123 }], [{ v: 1123 }], [{ wait: 500 }, { c: 1123 }], [{ timeout: 1e3, check: "none", wv: 1123 }], {}], [1, "auto_CH_svtplay.se_d39", 0, "^https?://(www\\.)?svtplay\\.se/", 10, [], [{ e: 1124 }], [{ v: 1124 }], [{ wait: 500 }, { c: 1124 }], [{ timeout: 1e3, check: "none", wv: 1124 }], {}], [1, "auto_CH_transn.ch_ygb", 0, "^https?://(www\\.)?transn\\.ch/", 10, [], [{ e: 1125 }], [{ v: 1125 }], [{ wait: 500 }, { c: 1125 }], [{ timeout: 1e3, check: "none", wv: 1125 }], {}], [1, "auto_CH_ubs-kidscup.ch_5oc", 0, "^https?://(www\\.)?ubs-kidscup\\.ch/", 10, [], [{ e: 1126 }], [{ v: 1126 }], [{ wait: 500 }, { c: 1126 }], [{ timeout: 1e3, check: "none", wv: 1126 }], {}], [1, "auto_CH_velofactory.ch_tpq", 0, "^https?://(www\\.)?velofactory\\.ch/", 10, [], [{ e: 1127 }], [{ v: 1127 }], [{ wait: 500 }, { c: 1127 }], [{ timeout: 1e3, check: "none", wv: 1127 }], {}], [1, "auto_DE_116117-termine.de_nwg", 0, "^https?://(www\\.)?116117-termine\\.de/", 10, [], [{ e: 1128 }], [{ v: 1128 }], [{ wait: 500 }, { c: 1128 }], [{ timeout: 1e3, check: "none", wv: 1128 }], {}], [1, "auto_DE_6relax.de_na9", 0, "^https?://(www\\.)?6relax\\.de/", 10, [], [{ e: 1129 }], [{ v: 1129 }], [{ wait: 500 }, { c: 1129 }], [{ timeout: 1e3, check: "none", wv: 1129 }], {}], [1, "auto_DE_accio.com_kdu", 0, "^https?://(www\\.)?accio\\.com/", 10, [], [{ e: 1130 }], [{ v: 1130 }], [{ wait: 500 }, { c: 1130 }], [{ timeout: 1e3, check: "none", wv: 1130 }], {}], [1, "auto_DE_aerztekammer-bw.de_rse", 0, "^https?://(www\\.)?aerztekammer-bw\\.de/", 10, [], [{ e: 1131 }], [{ v: 1131 }], [{ wait: 500 }, { c: 1131 }], [{ timeout: 1e3, check: "none", wv: 1131 }], {}], [1, "auto_DE_afd.de_tad", 0, "^https?://(www\\.)?afd\\.de/", 10, [], [{ e: 1132 }], [{ v: 1132 }], [{ wait: 500 }, { c: 1132 }], [{ timeout: 1e3, check: "none", wv: 1132 }], {}], [1, "auto_DE_aknw.de_wcz_+1", 0, "^https?://(www\\.)?aknw\\.de/|^https?://(www\\.)?regioentsorgung\\.de/", 10, [], [{ e: 1133 }], [{ v: 1133 }], [{ wait: 500 }, { c: 1133 }], [{ timeout: 1e3, check: "none", wv: 1133 }], {}], [1, "auto_DE_all-inkl.com_toh", 0, "^https?://(www\\.)?all-inkl\\.com/", 10, [], [{ e: 1134 }], [{ v: 1134 }], [{ wait: 500 }, { c: 1134 }], [{ timeout: 1e3, check: "none", wv: 1134 }], {}], [1, "auto_DE_almenrausch.at_c6z", 0, "^https?://(www\\.)?almenrausch\\.at/", 10, [], [{ e: 1135 }], [{ v: 1135 }], [{ wait: 500 }, { c: 1135 }], [{ timeout: 1e3, check: "none", wv: 1135 }], {}], [1, "auto_DE_alza.de_iq3", 0, "^https?://(www\\.)?alza\\.de/", 10, [], [{ e: 1136 }], [{ v: 1136 }], [{ wait: 500 }, { c: 1136 }], [{ timeout: 1e3, check: "none", wv: 1136 }], {}], [1, "auto_DE_ancestry.com_k1k", 0, "^https?://(www\\.)?ancestry\\.com/", 10, [], [{ e: 1137 }], [{ v: 1137 }], [{ wait: 500 }, { c: 1137 }], [{ timeout: 1e3, check: "none", wv: 1137 }], {}], [1, "auto_DE_apozilla.de_d1h", 0, "^https?://(www\\.)?apozilla\\.de/", 10, [], [{ e: 1138 }], [{ v: 1138 }], [{ wait: 500 }, { c: 1138 }], [{ timeout: 1e3, check: "none", wv: 1138 }], {}], [1, "auto_DE_ardplus.de_ue3", 0, "^https?://(www\\.)?ardplus\\.de/", 10, [], [{ e: 1139 }], [{ v: 1139 }], [{ wait: 500 }, { c: 1139 }], [{ timeout: 1e3, check: "none", wv: 1139 }], {}], [1, "auto_DE_axa.de_vyk", 0, "^https?://(www\\.)?axa\\.de/", 10, [], [{ e: 1140 }], [{ v: 1140 }], [{ wait: 500 }, { c: 1140 }], [{ timeout: 1e3, check: "none", wv: 1140 }], {}], [1, "auto_DE_backmarket.de_dzf", 0, "^https?://(www\\.)?backmarket\\.de/", 10, [], [{ e: 1141 }], [{ v: 1141 }], [{ wait: 500 }, { c: 1141 }], [{ timeout: 1e3, check: "none", wv: 1141 }], {}], [1, "auto_DE_bbbank.de_dcf_+15", 0, "^https?://(www\\.)?bbbank\\.de/|^https?://(www\\.)?diebank\\.de/|^https?://(www\\.)?genobroker\\.de/|^https?://(www\\.)?ligabank\\.de/|^https?://(www\\.)?pax-bkc\\.de/|^https?://(www\\.)?psd-berlin-brandenburg\\.de/|^https?://(www\\.)?psd-nuernberg\\.de/|^https?://(www\\.)?sparda-bank-hamburg\\.de/|^https?://(www\\.)?sparda-h\\.de/|^https?://(www\\.)?sparda-n\\.de/|^https?://(www\\.)?v-mn\\.de/|^https?://(www\\.)?vr-bayernmitte\\.de/|^https?://(www\\.)?vrbank-brs\\.de/|^https?://(www\\.)?vrbank-eg\\.de/|^https?://(www\\.)?vrbank-lb\\.de/|^https?://(www\\.)?wvb\\.de/", 10, [], [{ e: 1142 }], [{ v: 1142 }], [{ wait: 500 }, { c: 1142 }], [{ timeout: 1e3, check: "none", wv: 1142 }], {}], [1, "auto_DE_biunsinnorden.de_kwa_+1", 0, "^https?://(www\\.)?biunsinnorden\\.de/|^https?://(www\\.)?livegigs\\.de/", 10, [], [{ e: 1143 }], [{ v: 1143 }], [{ wait: 500 }, { c: 1143 }], [{ timeout: 1e3, check: "none", wv: 1143 }], {}], [1, "auto_DE_bmz.de_nss", 0, "^https?://(www\\.)?bmz\\.de/", 10, [], [{ e: 1144 }], [{ v: 1144 }], [{ wait: 500 }, { c: 1144 }], [{ timeout: 1e3, check: "none", wv: 1144 }], {}], [1, "auto_DE_buerklin.com_bya", 0, "^https?://(www\\.)?buerklin\\.com/", 10, [], [{ e: 1087 }], [{ v: 1087 }], [{ wait: 500 }, { c: 1087 }], [{ timeout: 1e3, check: "none", wv: 1087 }], {}], [1, "auto_DE_bundeswehrkarriere.de_g9g", 0, "^https?://(www\\.)?bundeswehrkarriere\\.de/", 10, [], [{ e: 1145 }], [{ v: 1145 }], [{ wait: 500 }, { c: 1145 }], [{ timeout: 1e3, check: "none", wv: 1145 }], {}], [1, "auto_DE_byak.de_dcj", 0, "^https?://(www\\.)?byak\\.de/", 10, [], [{ e: 1146 }], [{ v: 1146 }], [{ wait: 500 }, { c: 1146 }], [{ timeout: 1e3, check: "none", wv: 1146 }], {}], [1, "auto_DE_byte.fm_83l", 0, "^https?://(www\\.)?byte\\.fm/", 10, [], [{ e: 1147 }], [{ v: 1147 }], [{ wait: 500 }, { c: 1147 }], [{ timeout: 1e3, check: "none", wv: 1147 }], {}], [1, "auto_DE_camping-outdoorshop.de_oo4_+1", 0, "^https?://(www\\.)?camping-outdoorshop\\.de/|^https?://(www\\.)?elektro-wandelt\\.de/", 10, [], [{ e: 1135 }], [{ v: 1135 }], [{ wait: 500 }, { c: 1135 }], [{ timeout: 1e3, check: "none", wv: 1135 }], {}], [1, "auto_DE_club.auto-doc.at_6xj", 0, "^https?://(www\\.)?club\\.auto-doc\\.at/", 10, [], [{ e: 1148 }], [{ v: 1148 }], [{ wait: 500 }, { c: 1148 }], [{ timeout: 1e3, check: "none", wv: 1148 }], {}], [1, "auto_DE_daad.de_w45", 0, "^https?://(www\\.)?daad\\.de/", 10, [], [{ e: 1149 }], [{ v: 1149 }], [{ wait: 500 }, { c: 1149 }], [{ timeout: 1e3, check: "none", wv: 1149 }], {}], [1, "auto_DE_das-ist-drin.de_e12", 0, "^https?://(www\\.)?das-ist-drin\\.de/", 10, [], [{ e: 1150 }], [{ v: 1150 }], [{ wait: 500 }, { c: 1150 }], [{ timeout: 1e3, check: "none", wv: 1150 }], {}], [1, "auto_DE_de.accio.com_97o", 0, "^https?://(www\\.)?de\\.accio\\.com/", 10, [], [{ e: 1130 }], [{ v: 1130 }], [{ wait: 500 }, { c: 1130 }], [{ timeout: 1e3, check: "none", wv: 1130 }], {}], [1, "auto_DE_de.artprice.com_kfk", 0, "^https?://(www\\.)?de\\.artprice\\.com/", 10, [], [{ e: 1151 }], [{ v: 1151 }], [{ wait: 500 }, { c: 1151 }], [{ timeout: 1e3, check: "none", wv: 1151 }], {}], [1, "auto_DE_de.nothing.tech_0fr_+1", 0, "^https?://(www\\.)?de\\.nothing\\.tech/|^https?://(www\\.)?nothing\\.tech/", 10, [], [{ e: 1152 }], [{ v: 1152 }], [{ wait: 500 }, { c: 1152 }], [{ timeout: 1e3, check: "none", wv: 1152 }], {}], [1, "auto_DE_dekra.de_qrb", 0, "^https?://(www\\.)?dekra\\.de/", 10, [], [{ e: 1153 }], [{ v: 1153 }], [{ wait: 500 }, { c: 1153 }], [{ timeout: 1e3, check: "none", wv: 1153 }], {}], [1, "auto_DE_edelstahl-tuerklingel.de_375", 0, "^https?://(www\\.)?edelstahl-tuerklingel\\.de/", 10, [], [{ e: 1154 }], [{ v: 1154 }], [{ wait: 500 }, { c: 1154 }], [{ timeout: 1e3, check: "none", wv: 1154 }], {}], [1, "auto_DE_eezy.nrw_9aj", 0, "^https?://(www\\.)?eezy\\.nrw/", 10, [], [{ e: 1155 }], [{ v: 1155 }], [{ wait: 500 }, { c: 1155 }], [{ timeout: 1e3, check: "none", wv: 1155 }], {}], [1, "auto_DE_ernstings-family.de_xqf", 0, "^https?://(www\\.)?ernstings-family\\.de/", 10, [], [{ e: 1156 }], [{ v: 1156 }], [{ wait: 500 }, { c: 1156 }], [{ timeout: 1e3, check: "none", wv: 1156 }], {}], [1, "auto_DE_fcbinside.de_0d6", 0, "^https?://(www\\.)?fcbinside\\.de/", 10, [], [{ e: 1157 }], [{ v: 1157 }], [{ wait: 500 }, { c: 1157 }], [{ timeout: 1e3, check: "none", wv: 1157 }], {}], [1, "auto_DE_feierabend.de_kr4", 0, "^https?://(www\\.)?feierabend\\.de/", 10, [], [{ e: 1158 }], [{ v: 1158 }], [{ wait: 500 }, { c: 1158 }], [{ timeout: 1e3, check: "none", wv: 1158 }], {}], [1, "auto_DE_feser-graf.de_qz8", 0, "^https?://(www\\.)?feser-graf\\.de/", 10, [], [{ e: 1159 }], [{ v: 1159 }], [{ wait: 500 }, { c: 1159 }], [{ timeout: 1e3, check: "none", wv: 1159 }], {}], [1, "auto_DE_finanzpartner.de_13s", 0, "^https?://(www\\.)?finanzpartner\\.de/", 10, [], [{ e: 1160 }], [{ v: 1160 }], [{ wait: 500 }, { c: 1160 }], [{ timeout: 1e3, check: "none", wv: 1160 }], {}], [1, "auto_DE_gasometer.de_0xw", 0, "^https?://(www\\.)?gasometer\\.de/", 10, [], [{ e: 1161 }], [{ v: 1161 }], [{ wait: 500 }, { c: 1161 }], [{ timeout: 1e3, check: "none", wv: 1161 }], {}], [1, "auto_DE_hermoney.de_jsi", 0, "^https?://(www\\.)?hermoney\\.de/", 10, [], [{ e: 1162 }], [{ v: 1162 }], [{ wait: 500 }, { c: 1162 }], [{ timeout: 1e3, check: "none", wv: 1162 }], {}], [1, "auto_DE_hilfe.kleinanzeigen.de_44a_+1", 0, "^https?://(www\\.)?hilfe\\.kleinanzeigen\\.de/|^https?://(www\\.)?themen\\.kleinanzeigen\\.de/", 10, [], [{ e: 1163 }], [{ v: 1163 }], [{ wait: 500 }, { c: 1163 }], [{ timeout: 1e3, check: "none", wv: 1163 }], {}], [1, "auto_DE_howik.com_99g", 0, "^https?://(www\\.)?howik\\.com/", 10, [], [{ e: 1164 }], [{ v: 1164 }], [{ wait: 500 }, { c: 1164 }], [{ timeout: 1e3, check: "none", wv: 1164 }], {}], [1, "auto_DE_hu-berlin.de_sk6", 0, "^https?://(www\\.)?hu-berlin\\.de/", 10, [], [{ e: 1165 }], [{ v: 1165 }], [{ wait: 500 }, { c: 1165 }], [{ timeout: 1e3, check: "none", wv: 1165 }], {}], [1, "auto_DE_imd-berlin.de_6m1", 0, "^https?://(www\\.)?imd-berlin\\.de/", 10, [], [{ e: 1166 }], [{ v: 1166 }], [{ wait: 500 }, { c: 1166 }], [{ timeout: 1e3, check: "none", wv: 1166 }], {}], [1, "auto_DE_immobilien.sparkasse.de_zj7", 0, "^https?://(www\\.)?immobilien\\.sparkasse\\.de/", 10, [1167], [{ e: 1168 }], [{ v: 1168 }], [{ c: 1169 }], [{ timeout: 1e3, check: "none", wv: 1168 }], {}], [1, "auto_DE_impfen-info.de_am5_+1", 0, "^https?://(www\\.)?impfen-info\\.de/|^https?://(www\\.)?infektionsschutz\\.de/", 10, [], [{ e: 1170 }], [{ v: 1170 }], [{ wait: 500 }, { c: 1170 }], [{ timeout: 1e3, check: "none", wv: 1170 }], {}], [1, "auto_DE_jobvector.de_641", 0, "^https?://(www\\.)?jobvector\\.de/", 10, [], [{ e: 1171 }], [{ v: 1171 }], [{ wait: 500 }, { c: 1171 }], [{ timeout: 1e3, check: "none", wv: 1171 }], {}], [1, "auto_DE_kinsta.com_hc5", 0, "^https?://(www\\.)?kinsta\\.com/", 10, [], [{ e: 1172 }], [{ v: 1172 }], [{ wait: 500 }, { c: 1172 }], [{ timeout: 1e3, check: "none", wv: 1172 }], {}], [1, "auto_DE_kleineskraftwerk.de_fx7", 0, "^https?://(www\\.)?kleineskraftwerk\\.de/", 10, [], [{ e: 1173 }], [{ v: 1173 }], [{ wait: 500 }, { c: 1173 }], [{ timeout: 1e3, check: "none", wv: 1173 }], {}], [1, "auto_DE_kundenportal.m-net.de_y8l", 0, "^https?://(www\\.)?kundenportal\\.m-net\\.de/", 10, [], [{ e: 1122 }], [{ v: 1122 }], [{ wait: 500 }, { c: 1122 }], [{ timeout: 1e3, check: "none", wv: 1122 }], {}], [1, "auto_DE_kvhb.de_hhf", 0, "^https?://(www\\.)?kvhb\\.de/", 10, [], [{ e: 1174 }], [{ v: 1174 }], [{ wait: 500 }, { c: 1174 }], [{ timeout: 1e3, check: "none", wv: 1174 }], {}], [1, "auto_DE_la.spankbang.com_sva", 0, "^https?://(www\\.)?la\\.spankbang\\.com/", 10, [], [{ e: 1175 }], [{ v: 1175 }], [{ wait: 500 }, { c: 1175 }], [{ timeout: 1e3, check: "none", wv: 1175 }], {}], [1, "auto_DE_lbs.de_6zt", 0, "^https?://(www\\.)?lbs\\.de/", 10, [], [{ e: 1176 }], [{ v: 1176 }], [{ wait: 500 }, { c: 1176 }], [{ timeout: 1e3, check: "none", wv: 1176 }], {}], [1, "auto_DE_lenovo.com_xcv", 0, "^https?://(www\\.)?lenovo\\.com/", 10, [], [{ e: 1177 }], [{ v: 1177 }], [{ wait: 500 }, { c: 1177 }], [{ timeout: 1e3, check: "none", wv: 1177 }], {}], [1, "auto_DE_listando.de_c5i", 0, "^https?://(www\\.)?listando\\.de/", 10, [], [{ e: 1178 }], [{ v: 1178 }], [{ wait: 500 }, { c: 1178 }], [{ timeout: 1e3, check: "none", wv: 1178 }], {}], [1, "auto_DE_lite-magazin.de_e1s", 0, "^https?://(www\\.)?lite-magazin\\.de/", 10, [], [{ e: 1179 }], [{ v: 1179 }], [{ wait: 500 }, { c: 1179 }], [{ timeout: 1e3, check: "none", wv: 1179 }], {}], [1, "auto_DE_m.livejasmin.com_cvg", 0, "^https?://(www\\.)?livejasmin\\.com/", 10, [], [{ e: 1180 }], [{ v: 1180 }], [{ wait: 500 }, { c: 1180 }], [{ timeout: 1e3, check: "none", wv: 1180 }], {}], [1, "auto_DE_mhplus-krankenkasse.de_4xb", 0, "^https?://(www\\.)?mhplus-krankenkasse\\.de/", 10, [], [{ e: 1135 }], [{ v: 1135 }], [{ wait: 500 }, { c: 1135 }], [{ timeout: 1e3, check: "none", wv: 1135 }], {}], [1, "auto_DE_mitarbeiterservice.bayern.de_quh", 0, "^https?://(www\\.)?mitarbeiterservice\\.bayern\\.de/", 10, [], [{ e: 1181 }], [{ v: 1181 }], [{ wait: 500 }, { c: 1181 }], [{ timeout: 1e3, check: "none", wv: 1181 }], {}], [1, "auto_DE_mrmarvis.com_bo5", 0, "^https?://(www\\.)?mrmarvis\\.com/", 10, [], [{ e: 1182 }], [{ v: 1182 }], [{ wait: 500 }, { c: 1182 }], [{ timeout: 1e3, check: "none", wv: 1182 }], {}], [1, "auto_DE_nanu-nana.de_7my", 0, "^https?://(www\\.)?nanu-nana\\.de/", 10, [], [{ e: 1183 }], [{ v: 1183 }], [{ wait: 500 }, { c: 1183 }], [{ timeout: 1e3, check: "none", wv: 1183 }], {}], [1, "auto_DE_originalteile.mercedes-benz.de_tce", 0, "^https?://(www\\.)?originalteile\\.mercedes-benz\\.de/", 10, [], [{ e: 1184 }], [{ v: 1184 }], [{ wait: 500 }, { c: 1184 }], [{ timeout: 1e3, check: "none", wv: 1184 }], {}], [1, "auto_DE_parqet.com_6wm", 0, "^https?://(www\\.)?parqet\\.com/", 10, [], [{ e: 1185 }], [{ v: 1185 }], [{ wait: 500 }, { c: 1185 }], [{ timeout: 1e3, check: "none", wv: 1185 }], {}], [1, "auto_DE_pflanzenhof-online.de_au2", 0, "^https?://(www\\.)?pflanzenhof-online\\.de/", 10, [], [{ e: 1186 }], [{ v: 1186 }], [{ wait: 500 }, { c: 1186 }], [{ timeout: 1e3, check: "none", wv: 1186 }], {}], [1, "auto_DE_polizei.hessen.de_rsx", 0, "^https?://(www\\.)?polizei\\.hessen\\.de/", 10, [], [{ e: 1187 }], [{ v: 1187 }], [{ wait: 500 }, { c: 1187 }], [{ timeout: 1e3, check: "none", wv: 1187 }], {}], [1, "auto_DE_regierung.oberbayern.bayern.de_zx2_+2", 0, "^https?://(www\\.)?regierung\\.oberbayern\\.bayern\\.de/|^https?://(www\\.)?statistik\\.bayern\\.de/|^https?://(www\\.)?stmb\\.bayern\\.de/", 10, [], [{ e: 1188 }], [{ v: 1188 }], [{ wait: 500 }, { c: 1188 }], [{ timeout: 1e3, check: "none", wv: 1188 }], {}], [1, "auto_DE_roller.de_pjo", 0, "^https?://(www\\.)?roller\\.de/", 10, [], [{ e: 1189 }], [{ v: 1189 }], [{ wait: 500 }, { c: 1189 }], [{ timeout: 1e3, check: "none", wv: 1189 }], {}], [1, "auto_DE_rundfunkbeitrag.de_g4y", 0, "^https?://(www\\.)?rundfunkbeitrag\\.de/", 10, [], [{ e: 1190 }], [{ v: 1190 }], [{ wait: 500 }, { c: 1190 }], [{ timeout: 1e3, check: "none", wv: 1190 }], {}], [1, "auto_DE_schwabach.de_fjr", 0, "^https?://(www\\.)?schwabach\\.de/", 10, [], [{ e: 1191 }], [{ v: 1191 }], [{ wait: 500 }, { c: 1191 }], [{ timeout: 1e3, check: "none", wv: 1191 }], {}], [1, "auto_DE_schwaebisch-hall.de_0g1", 0, "^https?://(www\\.)?schwaebisch-hall\\.de/", 10, [], [{ e: 1192 }], [{ v: 1192 }], [{ wait: 500 }, { c: 1192 }], [{ timeout: 1e3, check: "none", wv: 1192 }], {}], [1, "auto_DE_sellercentral.amazon.de_xi0", 0, "^https?://(www\\.)?sellercentral\\.amazon\\.de/", 10, [], [{ e: 1193 }], [{ v: 1193 }], [{ wait: 500 }, { c: 1193 }], [{ timeout: 1e3, check: "none", wv: 1193 }], {}], [1, "auto_DE_sephora.de_exg", 0, "^https?://(www\\.)?sephora\\.de/", 10, [], [{ e: 1115 }], [{ v: 1115 }], [{ wait: 500 }, { c: 1115 }], [{ timeout: 1e3, check: "none", wv: 1115 }], {}], [1, "auto_DE_solarspeicher24.de_w5k", 0, "^https?://(www\\.)?solarspeicher24\\.de/", 10, [], [{ e: 1194 }], [{ v: 1194 }], [{ wait: 500 }, { c: 1194 }], [{ timeout: 1e3, check: "none", wv: 1194 }], {}], [1, "auto_DE_speedtest.vodafone.de_dha", 0, "^https?://(www\\.)?speedtest\\.vodafone\\.de/", 10, [], [{ e: 1195 }], [{ v: 1195 }], [{ wait: 500 }, { c: 1195 }], [{ timeout: 1e3, check: "none", wv: 1195 }], {}], [1, "auto_DE_steeltoyz.de_i51", 0, "^https?://(www\\.)?steeltoyz\\.de/", 10, [], [{ e: 1196 }], [{ v: 1196 }], [{ wait: 500 }, { c: 1196 }], [{ timeout: 1e3, check: "none", wv: 1196 }], {}], [1, "auto_DE_survival-kompass.de_kv6", 0, "^https?://(www\\.)?survival-kompass\\.de/", 10, [], [{ e: 1197 }], [{ v: 1197 }], [{ wait: 500 }, { c: 1197 }], [{ timeout: 1e3, check: "none", wv: 1197 }], {}], [1, "auto_DE_typografie.info_mnj", 0, "^https?://(www\\.)?typografie\\.info/", 10, [], [{ e: 1198 }], [{ v: 1198 }], [{ wait: 500 }, { c: 1198 }], [{ timeout: 1e3, check: "none", wv: 1198 }], {}], [1, "auto_DE_uni-hildesheim.de_7kj", 0, "^https?://(www\\.)?uni-hildesheim\\.de/", 10, [], [{ e: 1199 }], [{ v: 1199 }], [{ wait: 500 }, { c: 1199 }], [{ timeout: 1e3, check: "none", wv: 1199 }], {}], [1, "auto_DE_uni-mannheim.de_omi", 0, "^https?://(www\\.)?uni-mannheim\\.de/", 10, [], [{ e: 1200 }], [{ v: 1200 }], [{ wait: 500 }, { c: 1200 }], [{ timeout: 1e3, check: "none", wv: 1200 }], {}], [1, "auto_DE_variete.de_6cc", 0, "^https?://(www\\.)?variete\\.de/", 10, [], [{ e: 1201 }], [{ v: 1201 }], [{ wait: 500 }, { c: 1201 }], [{ timeout: 1e3, check: "none", wv: 1201 }], {}], [1, "auto_DE_wien.gv.at_mm2", 0, "^https?://(www\\.)?wien\\.gv\\.at/", 10, [], [{ e: 1202 }], [{ v: 1202 }], [{ wait: 500 }, { c: 1202 }], [{ timeout: 1e3, check: "none", wv: 1202 }], {}], [1, "auto_DE_wolt.com_jyq", 0, "^https?://(www\\.)?wolt\\.com/", 10, [], [{ e: 1203 }], [{ v: 1203 }], [{ wait: 500 }, { c: 1203 }], [{ timeout: 1e3, check: "none", wv: 1203 }], {}], [1, "auto_FR_3ds.com_pa7", 0, "^https?://(www\\.)?3ds\\.com/", 10, [], [{ e: 1204 }], [{ v: 1204 }], [{ wait: 500 }, { c: 1204 }], [{ timeout: 1e3, check: "none", wv: 1204 }], {}], [1, "auto_FR_aefinfo.fr_6r7", 0, "^https?://(www\\.)?aefinfo\\.fr/", 10, [], [{ e: 1205 }], [{ v: 1205 }], [{ wait: 500 }, { c: 1205 }], [{ timeout: 1e3, check: "none", wv: 1205 }], {}], [1, "auto_FR_alinea.com_d9k", 0, "^https?://(www\\.)?alinea\\.com/", 10, [], [{ e: 1206 }], [{ v: 1206 }], [{ wait: 500 }, { c: 1206 }], [{ timeout: 1e3, check: "none", wv: 1206 }], {}], [1, "auto_FR_alinea.com_gst", 0, "^https?://(www\\.)?alinea\\.com/", 10, [], [{ e: 1207 }], [{ v: 1207 }], [{ wait: 500 }, { c: 1207 }], [{ timeout: 1e3, check: "none", wv: 1207 }], {}], [1, "auto_FR_annuaire-inverse-france.com_4oi", 0, "^https?://(www\\.)?annuaire-inverse-france\\.com/", 10, [], [{ e: 1208 }], [{ v: 1208 }], [{ wait: 500 }, { c: 1208 }], [{ timeout: 1e3, check: "none", wv: 1208 }], {}], [1, "auto_FR_asp.gouv.fr_ytt", 0, "^https?://(www\\.)?asp\\.gouv\\.fr/", 10, [], [{ e: 1209 }], [{ v: 1209 }], [{ wait: 500 }, { c: 1209 }], [{ timeout: 1e3, check: "none", wv: 1209 }], {}], [1, "auto_FR_bd-adultes.com_nn2", 0, "^https?://(www\\.)?bd-adultes\\.com/", 10, [], [{ e: 1210 }], [{ v: 1210 }], [{ wait: 500 }, { c: 1210 }], [{ timeout: 1e3, check: "none", wv: 1210 }], {}], [1, "auto_FR_bobvoyeur.com_qm5", 0, "^https?://(www\\.)?bobvoyeur\\.com/", 10, [], [{ e: 1211 }], [{ v: 1211 }], [{ wait: 500 }, { c: 1211 }], [{ timeout: 1e3, check: "none", wv: 1211 }], {}], [1, "auto_FR_bpi.fr_l52", 0, "^https?://(www\\.)?bpi\\.fr/", 10, [], [{ e: 1061 }], [{ v: 1061 }], [{ wait: 500 }, { c: 1061 }], [{ timeout: 1e3, check: "none", wv: 1061 }], {}], [1, "auto_FR_chamrousse.com_i7u", 0, "^https?://(www\\.)?chamrousse\\.com/", 10, [], [{ e: 1212 }], [{ v: 1212 }], [{ wait: 500 }, { c: 1212 }], [{ timeout: 1e3, check: "none", wv: 1212 }], {}], [1, "auto_FR_charliehebdo.fr_smr", 0, "^https?://(www\\.)?charliehebdo\\.fr/", 10, [], [{ e: 1213 }], [{ v: 1213 }], [{ wait: 500 }, { c: 1213 }], [{ timeout: 1e3, check: "none", wv: 1213 }], {}], [1, "auto_FR_cite-sciences.fr_kcx", 0, "^https?://(www\\.)?cite-sciences\\.fr/", 10, [], [{ e: 1214 }], [{ v: 1214 }], [{ wait: 500 }, { c: 1214 }], [{ timeout: 1e3, check: "none", wv: 1214 }], {}], [1, "auto_FR_coe.int_cfo", 0, "^https?://(www\\.)?coe\\.int/", 10, [], [{ e: 1215 }], [{ v: 1215 }], [{ wait: 500 }, { c: 1215 }], [{ timeout: 1e3, check: "none", wv: 1215 }], {}], [1, "auto_FR_coutellerie-tourangelle.com_rcf", 0, "^https?://(www\\.)?coutellerie-tourangelle\\.com/", 10, [], [{ e: 1216 }], [{ v: 1216 }], [{ wait: 500 }, { c: 1216 }], [{ timeout: 1e3, check: "none", wv: 1216 }], {}], [1, "auto_FR_cre.fr_sd4", 0, "^https?://(www\\.)?cre\\.fr/", 10, [], [{ e: 1217 }], [{ v: 1217 }], [{ wait: 500 }, { c: 1217 }], [{ timeout: 1e3, check: "none", wv: 1217 }], {}], [1, "auto_FR_cybevasion.fr_jjp", 0, "^https?://(www\\.)?cybevasion\\.fr/", 10, [], [{ e: 1088 }], [{ v: 1088 }], [{ wait: 500 }, { c: 1088 }], [{ timeout: 1e3, check: "none", wv: 1088 }], {}], [1, "auto_FR_edumoov.com_sij", 0, "^https?://(www\\.)?edumoov\\.com/", 10, [], [{ e: 1218 }], [{ v: 1218 }], [{ wait: 500 }, { c: 1218 }], [{ timeout: 1e3, check: "none", wv: 1218 }], {}], [1, "auto_FR_engie-homeservices.fr_lyo", 0, "^https?://(www\\.)?engie-homeservices\\.fr/", 10, [], [{ e: 1219 }], [{ v: 1219 }], [{ wait: 500 }, { c: 1219 }], [{ timeout: 1e3, check: "none", wv: 1219 }], {}], [1, "auto_FR_es.xhamster.com_f69", 0, "^https?://(www\\.)?es\\.xhamster\\.com/", 10, [], [{ e: 1220 }], [{ v: 1220 }], [{ wait: 500 }, { c: 1220 }], [{ timeout: 1e3, check: "none", wv: 1220 }], {}], [1, "auto_FR_euro-expos.com_tok", 0, "^https?://(www\\.)?euro-expos\\.com/", 10, [], [{ e: 1221 }], [{ v: 1221 }], [{ wait: 500 }, { c: 1221 }], [{ timeout: 1e3, check: "none", wv: 1221 }], {}], [1, "auto_FR_fr.accio.com_zdn", 0, "^https?://(www\\.)?fr\\.accio\\.com/", 10, [], [{ e: 1130 }], [{ v: 1130 }], [{ wait: 500 }, { c: 1130 }], [{ timeout: 1e3, check: "none", wv: 1130 }], {}], [1, "auto_FR_fr.xgimi.com_fzb_+1", 0, "^https?://(www\\.)?fr\\.xgimi\\.com/|^https?://(www\\.)?leminor\\.fr/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_FR_glamuse.com_n32", 0, "^https?://(www\\.)?glamuse\\.com/", 10, [], [{ e: 1222 }], [{ v: 1222 }], [{ wait: 500 }, { c: 1222 }], [{ timeout: 1e3, check: "none", wv: 1222 }], {}], [1, "auto_FR_gmf.fr_pxt", 0, "^https?://(www\\.)?gmf\\.fr/", 10, [], [{ e: 1223 }], [{ v: 1223 }], [{ wait: 500 }, { c: 1223 }], [{ timeout: 1e3, check: "none", wv: 1223 }], {}], [1, "auto_FR_gov.br_n2f", 0, "^https?://(www\\.)?gov\\.br/", 10, [], [{ e: 1224 }], [{ v: 1224 }], [{ wait: 500 }, { c: 1224 }], [{ timeout: 1e3, check: "none", wv: 1224 }], {}], [1, "auto_FR_greengo.voyage_fg3", 0, "^https?://(www\\.)?greengo\\.voyage/", 10, [], [{ e: 1225 }], [{ v: 1225 }], [{ wait: 500 }, { c: 1225 }], [{ timeout: 1e3, check: "none", wv: 1225 }], {}], [1, "auto_FR_haproxy.com_arh", 0, "^https?://(www\\.)?haproxy\\.com/", 10, [], [{ e: 1226 }], [{ v: 1226 }], [{ wait: 500 }, { c: 1226 }], [{ timeout: 1e3, check: "none", wv: 1226 }], {}], [1, "auto_FR_interencheres.com_c67", 0, "^https?://(www\\.)?interencheres\\.com/", 10, [], [{ e: 1221 }], [{ v: 1221 }], [{ wait: 500 }, { c: 1221 }], [{ timeout: 1e3, check: "none", wv: 1221 }], {}], [1, "auto_FR_ita.xhamster.com_jhk", 0, "^https?://(www\\.)?ita\\.xhamster\\.com/", 10, [], [{ e: 1220 }], [{ v: 1220 }], [{ wait: 500 }, { c: 1220 }], [{ timeout: 1e3, check: "none", wv: 1220 }], {}], [1, "auto_FR_kobo.com_ajz", 0, "^https?://(www\\.)?kobo\\.com/", 10, [], [{ e: 1227 }], [{ v: 1227 }], [{ wait: 500 }, { c: 1227 }], [{ timeout: 1e3, check: "none", wv: 1227 }], {}], [1, "auto_FR_lalibrairie.com_0lt", 0, "^https?://(www\\.)?lalibrairie\\.com/", 10, [], [{ e: 1228 }], [{ v: 1228 }], [{ wait: 500 }, { c: 1228 }], [{ timeout: 1e3, check: "none", wv: 1228 }], {}], [1, "auto_FR_leclercvoyages.com_2o4", 0, "^https?://(www\\.)?leclercvoyages\\.com/", 10, [], [{ e: 1223 }], [{ v: 1223 }], [{ wait: 500 }, { c: 1223 }], [{ timeout: 1e3, check: "none", wv: 1223 }], {}], [1, "auto_FR_lesprosdelapetiteenfance.fr_bng", 0, "^https?://(www\\.)?lesprosdelapetiteenfance\\.fr/", 10, [], [{ e: 1091 }], [{ v: 1091 }], [{ wait: 500 }, { c: 1091 }], [{ timeout: 1e3, check: "none", wv: 1091 }], {}], [1, "auto_FR_ludum.fr_gl5", 0, "^https?://(www\\.)?ludum\\.fr/", 10, [], [{ e: 1229 }], [{ v: 1229 }], [{ wait: 500 }, { c: 1229 }], [{ timeout: 1e3, check: "none", wv: 1229 }], {}], [1, "auto_FR_maboussoleaidants.fr_f8f", 0, "^https?://(www\\.)?maboussoleaidants\\.fr/", 10, [], [{ e: 1230 }], [{ v: 1230 }], [{ wait: 500 }, { c: 1230 }], [{ timeout: 1e3, check: "none", wv: 1230 }], {}], [1, "auto_FR_magellan-bio.fr_5xr", 0, "^https?://(www\\.)?magellan-bio\\.fr/", 10, [], [{ e: 1231 }], [{ v: 1231 }], [{ wait: 500 }, { c: 1231 }], [{ timeout: 1e3, check: "none", wv: 1231 }], {}], [1, "auto_FR_manuels.solutions_3gb", 0, "^https?://(www\\.)?manuels\\.solutions/", 10, [], [{ e: 1232 }], [{ v: 1232 }], [{ wait: 500 }, { c: 1232 }], [{ timeout: 1e3, check: "none", wv: 1232 }], {}], [1, "auto_FR_maty.com_2v7", 0, "^https?://(www\\.)?maty\\.com/", 10, [], [{ e: 1122 }], [{ v: 1122 }], [{ wait: 500 }, { c: 1122 }], [{ timeout: 1e3, check: "none", wv: 1122 }], {}], [1, "auto_FR_mawaqit.net_0cw", 0, "^https?://(www\\.)?mawaqit\\.net/", 10, [], [{ e: 1233 }], [{ v: 1233 }], [{ wait: 500 }, { c: 1233 }], [{ timeout: 1e3, check: "none", wv: 1233 }], {}], [1, "auto_FR_meformerenregion.fr_64b", 0, "^https?://(www\\.)?meformerenregion\\.fr/", 10, [], [{ e: 1234 }], [{ v: 1234 }], [{ wait: 500 }, { c: 1234 }], [{ timeout: 1e3, check: "none", wv: 1234 }], {}], [1, "auto_FR_mesinfos.fr_gt2", 0, "^https?://(www\\.)?mesinfos\\.fr/", 10, [], [{ e: 1235 }], [{ v: 1235 }], [{ wait: 500 }, { c: 1235 }], [{ timeout: 1e3, check: "none", wv: 1235 }], {}], [1, "auto_FR_naval-group.com_yzx", 0, "^https?://(www\\.)?naval-group\\.com/", 10, [], [{ e: 1091 }], [{ v: 1091 }], [{ wait: 500 }, { c: 1091 }], [{ timeout: 1e3, check: "none", wv: 1091 }], {}], [1, "auto_FR_norauto.fr_mbi", 0, "^https?://(www\\.)?norauto\\.fr/", 10, [], [{ e: 1122 }], [{ v: 1122 }], [{ wait: 500 }, { c: 1122 }], [{ timeout: 1e3, check: "none", wv: 1122 }], {}], [1, "auto_FR_nouslib.com_1tl", 0, "^https?://(www\\.)?nouslib\\.com/", 10, [], [{ e: 1236 }], [{ v: 1236 }], [{ wait: 500 }, { c: 1236 }], [{ timeout: 1e3, check: "none", wv: 1236 }], {}], [1, "auto_FR_oceane.breizhgo.bzh_g7u", 0, "^https?://(www\\.)?oceane\\.breizhgo\\.bzh/", 10, [], [{ e: 1237 }], [{ v: 1237 }], [{ wait: 500 }, { c: 1237 }], [{ timeout: 1e3, check: "none", wv: 1237 }], {}], [1, "auto_FR_pfg.fr_j2d_+1", 0, "^https?://(www\\.)?pfg\\.fr/|^https?://(www\\.)?pointp\\.fr/", 10, [], [{ e: 1223 }], [{ v: 1223 }], [{ wait: 500 }, { c: 1223 }], [{ timeout: 1e3, check: "none", wv: 1223 }], {}], [1, "auto_FR_picwish.com_l0k", 0, "^https?://(www\\.)?picwish\\.com/", 10, [], [{ e: 1238 }], [{ v: 1238 }], [{ wait: 500 }, { c: 1238 }], [{ timeout: 1e3, check: "none", wv: 1238 }], {}], [1, "auto_FR_platform.openai.com_nyz", 0, "^https?://(www\\.)?platform\\.openai\\.com/", 10, [], [{ e: 1239 }], [{ v: 1239 }], [{ wait: 500 }, { c: 1239 }], [{ timeout: 1e3, check: "none", wv: 1239 }], {}], [1, "auto_FR_pointdevente.parionssport.fdj.fr_9uh", 0, "^https?://(www\\.)?pointdevente\\.parionssport\\.fdj\\.fr/", 10, [], [{ e: 1176 }], [{ v: 1176 }], [{ wait: 500 }, { c: 1176 }], [{ timeout: 1e3, check: "none", wv: 1176 }], {}], [1, "auto_FR_politis.fr_g33", 0, "^https?://(www\\.)?politis\\.fr/", 10, [], [{ e: 1240 }], [{ v: 1240 }], [{ wait: 500 }, { c: 1240 }], [{ timeout: 1e3, check: "none", wv: 1240 }], {}], [1, "auto_FR_pretto.fr_kb5", 0, "^https?://(www\\.)?pretto\\.fr/", 10, [], [{ e: 1241 }], [{ v: 1241 }], [{ wait: 500 }, { c: 1241 }], [{ timeout: 1e3, check: "none", wv: 1241 }], {}], [1, "auto_FR_privateaser.com_uco", 0, "^https?://(www\\.)?privateaser\\.com/", 10, [], [{ e: 1242 }], [{ v: 1242 }], [{ wait: 500 }, { c: 1242 }], [{ timeout: 1e3, check: "none", wv: 1242 }], {}], [1, "auto_FR_pro.inserm.fr_omt", 0, "^https?://(www\\.)?pro\\.inserm\\.fr/", 10, [], [{ e: 1061 }], [{ v: 1061 }], [{ wait: 500 }, { c: 1061 }], [{ timeout: 1e3, check: "none", wv: 1061 }], {}], [1, "auto_FR_proantic.com_oyg", 0, "^https?://(www\\.)?proantic\\.com/", 10, [], [{ e: 1243 }], [{ v: 1243 }], [{ wait: 500 }, { c: 1243 }], [{ timeout: 1e3, check: "none", wv: 1243 }], {}], [1, "auto_FR_revue-histoire.fr_jex", 0, "^https?://(www\\.)?revue-histoire\\.fr/", 10, [], [{ e: 1244 }], [{ v: 1244 }], [{ wait: 500 }, { c: 1244 }], [{ timeout: 1e3, check: "none", wv: 1244 }], {}], [1, "auto_FR_rhinoshield.fr_k5l", 0, "^https?://(www\\.)?rhinoshield\\.fr/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_FR_sephora.fr_k3l", 0, "^https?://(www\\.)?sephora\\.fr/", 10, [], [{ e: 1115 }], [{ v: 1115 }], [{ wait: 500 }, { c: 1115 }], [{ timeout: 1e3, check: "none", wv: 1115 }], {}], [1, "auto_FR_xhamster.desi_pv1_+1", 0, "^https?://(www\\.)?xhamster\\.desi/|^https?://(www\\.)?xhamster3\\.com/", 10, [], [{ e: 1220 }], [{ v: 1220 }], [{ wait: 500 }, { c: 1220 }], [{ timeout: 1e3, check: "none", wv: 1220 }], {}], [1, "auto_GB_3djake.uk_0", 0, "^https?://(www\\.)?3djake\\.uk/", 10, [], [{ e: 1245 }], [{ v: 1245 }], [{ c: 1245 }], [], {}], [1, "auto_GB_actionfraud.org.uk_92k", 0, "^https?://(www\\.)?actionfraud\\.org\\.uk/", 10, [], [{ e: 1246 }], [{ v: 1246 }], [{ wait: 500 }, { c: 1246 }], [{ timeout: 1e3, check: "none", wv: 1246 }], {}], [1, "auto_GB_ancestry.com_0", 0, "^https?://(www\\.)?ancestry\\.com/", 10, [], [{ e: 1247 }], [{ v: 1247 }], [{ wait: 500 }, { c: 1247 }], [{ timeout: 1e3, check: "none", wv: 1247 }], {}], [1, "auto_GB_arte.tv_0", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 1248 }], [{ v: 1248 }], [{ c: 1248 }], [], {}], [1, "auto_GB_bensnaturalhealth.co.uk_0", 0, "^https?://(www\\.)?bensnaturalhealth\\.co\\.uk/", 10, [], [{ e: 1249 }], [{ v: 1249 }], [{ c: 1249 }], [], {}], [1, "auto_GB_bike24.com_0", 0, "^https?://(www\\.)?bike24\\.com/", 10, [], [{ e: 1250 }], [{ v: 1250 }], [{ c: 1250 }], [], {}], [1, "auto_GB_brazzers.com_0", 0, "^https?://(www\\.)?brazzers\\.com/", 10, [], [{ e: 1251 }], [{ v: 1251 }], [{ c: 1251 }], [], {}], [1, "auto_GB_bricksandlogic.co.uk_o5o", 0, "^https?://(www\\.)?bricksandlogic\\.co\\.uk/", 10, [], [{ e: 1252 }], [{ v: 1252 }], [{ wait: 500 }, { c: 1252 }], [{ timeout: 1e3, check: "none", wv: 1252 }], {}], [1, "auto_GB_brightondome.org_iz9", 0, "^https?://(www\\.)?brightondome\\.org/", 10, [], [{ e: 1253 }], [{ v: 1253 }], [{ wait: 500 }, { c: 1253 }], [{ timeout: 1e3, check: "none", wv: 1253 }], {}], [1, "auto_GB_businessclass.com_0", 0, "^https?://(www\\.)?businessclass\\.com/", 10, [], [{ e: 1254 }], [{ v: 1254 }], [{ c: 1254 }], [], {}], [1, "auto_GB_capcut.com_0", 0, "^https?://(www\\.)?capcut\\.com/", 10, [], [{ e: 1255 }], [{ v: 1255 }], [{ c: 1255 }], [], {}], [1, "auto_GB_cardmarket.com_oxh", 0, "^https?://(www\\.)?cardmarket\\.com/", 10, [], [{ e: 1256 }], [{ v: 1256 }], [{ wait: 500 }, { c: 1256 }], [{ timeout: 1e3, check: "none", wv: 1256 }], {}], [1, "auto_GB_catawiki.com_0", 0, "^https?://(www\\.)?catawiki\\.com/", 10, [], [{ e: 1257 }], [{ v: 1257 }], [{ c: 1257 }], [], {}], [1, "auto_GB_charlesclinkard.co.uk_0", 0, "^https?://(www\\.)?charlesclinkard\\.co\\.uk/", 10, [], [{ e: 1258 }], [{ v: 1258 }], [{ wait: 500 }, { c: 1258 }], [{ timeout: 1e3, check: "none", wv: 1258 }], {}], [1, "auto_GB_chilternseeds.co.uk_0", 0, "^https?://(www\\.)?chilternseeds\\.co\\.uk/", 10, [], [{ e: 1259 }], [{ v: 1259 }], [{ wait: 500 }, { c: 1259 }], [{ timeout: 1e3, check: "none", wv: 1259 }], {}], [1, "auto_GB_chrono24.com_0", 0, "^https?://(www\\.)?chrono24\\.com/", 10, [], [{ e: 1260 }], [{ v: 1260 }], [{ c: 1260 }], [], {}], [1, "auto_GB_cpfc.co.uk_0", 0, "^https?://(www\\.)?cpfc\\.co\\.uk/", 10, [], [{ e: 1261 }], [{ v: 1261 }], [{ c: 1261 }], [], {}], [1, "auto_GB_deezer.com_0", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 1262 }], [{ v: 1262 }], [{ c: 1262 }], [], {}], [1, "auto_GB_edinburghcastle.scot_h2e", 0, "^https?://(www\\.)?edinburghcastle\\.scot/", 10, [], [{ e: 1263 }], [{ v: 1263 }], [{ wait: 500 }, { c: 1263 }], [{ timeout: 1e3, check: "none", wv: 1263 }], {}], [1, "auto_GB_europarl.europa.eu_0", 0, "^https?://(www\\.)?europarl\\.europa\\.eu/", 10, [], [{ e: 1264 }], [{ v: 1264 }], [{ c: 1264 }], [], {}], [1, "auto_GB_everysaving.co.uk_38t", 0, "^https?://(www\\.)?everysaving\\.co\\.uk/", 10, [], [{ e: 1265 }], [{ v: 1265 }], [{ c: 1265 }], [], {}], [1, "auto_GB_ewrc-results.com_y5f", 0, "^https?://(www\\.)?ewrc-results\\.com/", 10, [], [{ e: 1266 }], [{ v: 1266 }], [{ wait: 500 }, { c: 1266 }], [{ timeout: 1e3, check: "none", wv: 1266 }], {}], [1, "auto_GB_f6s.com_221", 0, "^https?://(www\\.)?f6s\\.com/", 10, [], [{ e: 1267 }], [{ v: 1267 }], [{ wait: 500 }, { c: 1267 }], [{ timeout: 1e3, check: "none", wv: 1267 }], {}], [1, "auto_GB_faphouse.com_0", 0, "^https?://(www\\.)?faphouse\\.com/", 10, [], [{ e: 1268 }], [{ v: 1268 }], [{ c: 1268 }], [], {}], [1, "auto_GB_farmergracy.co.uk_dl3", 0, "^https?://(www\\.)?farmergracy\\.co\\.uk/", 10, [], [{ e: 1269 }], [{ v: 1269 }], [{ wait: 500 }, { c: 1269 }], [{ timeout: 1e3, check: "none", wv: 1269 }], {}], [1, "auto_GB_fca.org.uk_9p9", 0, "^https?://(www\\.)?fca\\.org\\.uk/", 10, [], [{ e: 1270 }], [{ v: 1270 }], [{ wait: 500 }, { c: 1270 }], [{ timeout: 1e3, check: "none", wv: 1270 }], {}], [1, "auto_GB_forum.affinity.serif.com_0", 0, "^https?://(www\\.)?forum\\.affinity\\.serif\\.com/", 10, [], [{ e: 1271 }], [{ v: 1271 }], [{ c: 1271 }], [], {}], [1, "auto_GB_garden4less.co.uk_0", 0, "^https?://(www\\.)?garden4less\\.co\\.uk/", 10, [], [{ e: 1272 }], [{ v: 1272 }], [{ c: 1272 }], [], {}], [1, "auto_GB_glassesdirect.co.uk_bt9", 0, "^https?://(www\\.)?glassesdirect\\.co\\.uk/", 10, [], [{ e: 1273 }], [{ v: 1273 }], [{ wait: 500 }, { c: 1273 }], [{ timeout: 1e3, check: "none", wv: 1273 }], {}], [1, "auto_GB_handbook.fca.org.uk_0", 0, "^https?://(www\\.)?handbook\\.fca\\.org\\.uk/", 10, [], [{ e: 1274 }], [{ v: 1274 }], [{ wait: 500 }, { c: 1274 }], [{ timeout: 1e3, check: "none", wv: 1274 }], {}], [1, "auto_GB_historicenvironment.scot_0", 0, "^https?://(www\\.)?historicenvironment\\.scot/", 10, [], [{ e: 1275 }], [{ v: 1275 }], [{ c: 1275 }], [], {}], [1, "auto_GB_ionos.co.uk_c0a", 0, "^https?://(www\\.)?ionos\\.co\\.uk/", 10, [], [{ e: 1276 }], [{ v: 1276 }], [{ wait: 500 }, { c: 1276 }], [{ timeout: 1e3, check: "none", wv: 1276 }], {}], [1, "auto_GB_kick.com_0", 0, "^https?://(www\\.)?kick\\.com/", 10, [], [{ e: 1277 }], [{ v: 1277 }], [{ c: 1277 }], [], {}], [1, "auto_GB_kinopoisk.ru_0", 0, "^https?://(www\\.)?kinopoisk\\.ru/", 10, [], [{ e: 1278 }], [{ v: 1278 }], [{ wait: 500 }, { c: 1278 }], [{ timeout: 1e3, check: "none", wv: 1278 }], {}], [1, "auto_GB_kirklees.gov.uk_0", 0, "^https?://(www\\.)?kirklees\\.gov\\.uk/", 10, [], [{ e: 1279 }], [{ v: 1279 }], [{ c: 1279 }], [], {}], [1, "auto_GB_lancaster.ac.uk_0", 0, "^https?://(www\\.)?lancaster\\.ac\\.uk/", 10, [], [{ e: 1280 }], [{ v: 1280 }], [{ c: 1280 }], [], {}], [1, "auto_GB_lustery.com_0", 0, "^https?://(www\\.)?lustery\\.com/", 10, [], [{ e: 1281 }], [{ v: 1281 }], [{ c: 1281 }], [], {}], [1, "auto_GB_m.yandex.com_0_+2", 0, "^https?://(www\\.)?m\\.yandex\\.com/|^https?://(www\\.)?online\\.yandex\\.com/|^https?://(www\\.)?xmlsearch\\.yandex\\.ru/", 10, [], [{ e: 1282 }], [{ v: 1282 }], [{ text: "Allow essential cookies", c: 1282 }], [], {}], [1, "auto_GB_mypharmacy.co.uk_0", 0, "^https?://(www\\.)?mypharmacy\\.co\\.uk/", 10, [], [{ e: 1283 }], [{ v: 1283 }], [{ text: "DENY ALL", c: 1283 }], [], {}], [1, "auto_GB_onestream.co.uk_dpx", 0, "^https?://(www\\.)?onestream\\.co\\.uk/", 10, [], [{ e: 1284 }], [{ v: 1284 }], [{ wait: 500 }, { c: 1284 }], [{ timeout: 1e3, check: "none", wv: 1284 }], {}], [1, "auto_GB_outfox.energy_6ux", 0, "^https?://(www\\.)?outfox\\.energy/", 10, [], [{ e: 1285 }], [{ v: 1285 }], [{ wait: 500 }, { c: 1285 }], [{ timeout: 1e3, check: "none", wv: 1285 }], {}], [1, "auto_GB_parliamentlive.tv_r3v", 0, "^https?://(www\\.)?parliamentlive\\.tv/", 10, [], [{ e: 1286 }], [{ v: 1286 }], [{ c: 1286 }], [], {}], [1, "auto_GB_partscentre.co.uk_s70", 0, "^https?://(www\\.)?partscentre\\.co\\.uk/", 10, [], [{ e: 1287 }], [{ v: 1287 }], [{ wait: 500 }, { c: 1287 }], [{ timeout: 1e3, check: "none", wv: 1287 }], {}], [1, "auto_GB_plumbingworld.co.uk_vmi", 0, "^https?://(www\\.)?plumbingworld\\.co\\.uk/", 10, [], [{ e: 1288 }], [{ v: 1288 }], [{ wait: 500 }, { c: 1288 }], [{ timeout: 1e3, check: "none", wv: 1288 }], {}], [1, "auto_GB_reading.gov.uk_0", 0, "^https?://(www\\.)?reading\\.gov\\.uk/", 10, [], [{ e: 1289 }], [{ v: 1289 }], [{ wait: 500 }, { c: 1289 }], [{ timeout: 1e3, check: "none", wv: 1289 }], {}], [1, "auto_GB_shopify.com_0", 0, "^https?://(www\\.)?shopify\\.com/", 10, [], [{ e: 1290 }], [{ v: 1290 }], [{ c: 1290 }], [], {}], [1, "auto_GB_sso.passport.yandex.ru_0_+4", 0, "^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?yandex\\.com/", 10, [], [{ e: 1278 }], [{ v: 1278 }], [{ c: 1278 }], [], {}], [1, "auto_GB_stoneacre.co.uk_73c", 0, "^https?://(www\\.)?stoneacre\\.co\\.uk/", 10, [], [{ e: 1291 }], [{ v: 1291 }], [{ wait: 500 }, { c: 1291 }], [{ timeout: 1e3, check: "none", wv: 1291 }], {}], [1, "auto_GB_supremecourt.uk_0", 0, "^https?://(www\\.)?supremecourt\\.uk/", 10, [], [{ e: 1292 }], [{ v: 1292 }], [{ c: 1292 }], [{ timeout: 1e3, check: "none", wv: 1292 }], {}], [1, "auto_GB_thebatteryshop.co.uk_0", 0, "^https?://(www\\.)?thebatteryshop\\.co\\.uk/", 10, [], [{ e: 1293 }], [{ v: 1293 }], [{ text: "Reject All", c: 1293 }], [], {}], [1, "auto_GB_thebushcraftstore.co.uk_0_+1", 0, "^https?://(www\\.)?thebushcraftstore\\.co\\.uk/|^https?://(www\\.)?thewoolfactory\\.co\\.uk/", 10, [], [{ e: 1294 }], [{ v: 1294 }], [{ wait: 500 }, { c: 1294 }], [{ timeout: 1e3, check: "none", wv: 1294 }], {}], [1, "auto_GB_trove.scot_xtg", 0, "^https?://(www\\.)?trove\\.scot/", 10, [], [{ e: 1295 }], [{ v: 1295 }], [{ wait: 500 }, { c: 1295 }], [{ timeout: 1e3, check: "none", wv: 1295 }], {}], [1, "auto_GB_truecaller.com_0", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 1296 }], [{ v: 1296 }], [{ c: 1296 }], [], {}], [1, "auto_GB_vetuk.co.uk_0", 0, "^https?://(www\\.)?vetuk\\.co\\.uk/", 10, [], [{ e: 1297 }], [{ v: 1297 }], [{ wait: 500 }, { c: 1297 }], [{ timeout: 1e3, check: "none", wv: 1297 }], {}], [1, "auto_GB_virgin.com_0", 0, "^https?://(www\\.)?virgin\\.com/", 10, [], [{ e: 1298 }], [{ v: 1298 }], [{ wait: 500 }, { c: 1298 }], [{ timeout: 1e3, check: "none", wv: 1298 }], {}], [1, "auto_GB_vrhump.com_pb5", 0, "^https?://(www\\.)?vrhump\\.com/", 10, [], [{ e: 1299 }], [{ v: 1299 }], [{ wait: 500 }, { c: 1299 }], [{ timeout: 1e3, check: "none", wv: 1299 }], {}], [1, "auto_GB_weldricks.co.uk_0", 0, "^https?://(www\\.)?weldricks\\.co\\.uk/", 10, [], [{ e: 1300 }], [{ v: 1300 }], [{ wait: 500 }, { c: 1300 }], [{ timeout: 1e3, check: "none", wv: 1300 }], {}], [1, "auto_NL_3cx.com_0mf", 0, "^https?://(www\\.)?3cx\\.com/", 10, [], [{ e: 1301 }], [{ v: 1301 }], [{ wait: 500 }, { c: 1301 }], [{ timeout: 1e3, check: "none", wv: 1301 }], {}], [1, "auto_NL_aegon.nl_y2y", 0, "^https?://(www\\.)?aegon\\.nl/", 10, [], [{ e: 1302 }], [{ v: 1302 }], [{ wait: 500 }, { c: 1302 }], [{ timeout: 1e3, check: "none", wv: 1302 }], {}], [1, "auto_NL_app.chatgirl.nl_uqi", 0, "^https?://(www\\.)?app\\.chatgirl\\.nl/", 10, [], [{ e: 1303 }], [{ v: 1303 }], [{ wait: 500 }, { c: 1303 }], [{ timeout: 1e3, check: "none", wv: 1303 }], {}], [1, "auto_NL_asnbank.nl_e28_+1", 0, "^https?://(www\\.)?asnbank\\.nl/|^https?://(www\\.)?blgwonen\\.nl/", 10, [], [{ e: 1304 }], [{ v: 1304 }], [{ wait: 500 }, { c: 1304 }], [{ timeout: 1e3, check: "none", wv: 1304 }], {}], [1, "auto_NL_beekman.nl_f9e", 0, "^https?://(www\\.)?beekman\\.nl/", 10, [], [{ e: 1305 }], [{ v: 1305 }], [{ wait: 500 }, { c: 1305 }], [{ timeout: 1e3, check: "none", wv: 1305 }], {}], [1, "auto_NL_berivita.com_q3z", 0, "^https?://(www\\.)?berivita\\.com/", 10, [], [{ e: 1306 }], [{ v: 1306 }], [{ wait: 500 }, { c: 1306 }], [{ timeout: 1e3, check: "none", wv: 1306 }], {}], [1, "auto_NL_bibliotheekaanzet.nl_mh6", 0, "^https?://(www\\.)?bibliotheekaanzet\\.nl/", 10, [], [{ e: 1307 }], [{ v: 1307 }], [{ wait: 500 }, { c: 1307 }], [{ timeout: 1e3, check: "none", wv: 1307 }], {}], [1, "auto_NL_bk.nl_tx4", 0, "^https?://(www\\.)?bk\\.nl/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_NL_bonprix.nl_ovi", 0, "^https?://(www\\.)?bonprix\\.nl/", 10, [], [{ e: 1308 }], [{ v: 1308 }], [{ wait: 500 }, { c: 1308 }], [{ timeout: 1e3, check: "none", wv: 1308 }], {}], [1, "auto_NL_braumarkt.com_wfi", 0, "^https?://(www\\.)?braumarkt\\.com/", 10, [], [{ e: 1184 }], [{ v: 1184 }], [{ wait: 500 }, { c: 1184 }], [{ timeout: 1e3, check: "none", wv: 1184 }], {}], [1, "auto_NL_brillen24.nl_08c", 0, "^https?://(www\\.)?brillen24\\.nl/", 10, [], [{ e: 1309 }], [{ v: 1309 }], [{ wait: 500 }, { c: 1309 }], [{ timeout: 1e3, check: "none", wv: 1309 }], {}], [1, "auto_NL_chasse.nl_gs1", 0, "^https?://(www\\.)?chasse\\.nl/", 10, [], [{ e: 1310 }], [{ v: 1310 }], [{ wait: 500 }, { c: 1310 }], [{ timeout: 1e3, check: "none", wv: 1310 }], {}], [1, "auto_NL_cheaptickets.nl_cwt", 0, "^https?://(www\\.)?cheaptickets\\.nl/", 10, [], [{ e: 1311 }], [{ v: 1311 }], [{ wait: 500 }, { c: 1311 }], [{ timeout: 1e3, check: "none", wv: 1311 }], {}], [1, "auto_NL_chillplanet.nl_xcp", 0, "^https?://(www\\.)?chillplanet\\.nl/", 10, [], [{ e: 1312 }], [{ v: 1312 }], [{ wait: 500 }, { c: 1312 }], [{ timeout: 1e3, check: "none", wv: 1312 }], {}], [1, "auto_NL_chrono24.nl_dco", 0, "^https?://(www\\.)?chrono24\\.nl/", 10, [], [{ e: 1089 }], [{ v: 1089 }], [{ wait: 500 }, { c: 1089 }], [{ timeout: 1e3, check: "none", wv: 1089 }], {}], [1, "auto_NL_clinicaldiagnostics.nl_pmu", 0, "^https?://(www\\.)?clinicaldiagnostics\\.nl/", 10, [], [{ e: 1313 }], [{ v: 1313 }], [{ wait: 500 }, { c: 1313 }], [{ timeout: 1e3, check: "none", wv: 1313 }], {}], [1, "auto_NL_consumentenbond.nl_53g", 0, "^https?://(www\\.)?consumentenbond\\.nl/", 10, [], [{ e: 1314 }], [{ v: 1314 }], [{ wait: 500 }, { c: 1314 }], [{ timeout: 1e3, check: "none", wv: 1314 }], {}], [1, "auto_NL_denboschregion.nl_4x6", 0, "^https?://(www\\.)?denboschregion\\.nl/", 10, [], [{ e: 1315 }], [{ v: 1315 }], [{ wait: 500 }, { c: 1315 }], [{ timeout: 1e3, check: "none", wv: 1315 }], {}], [1, "auto_NL_denieuwebibliotheek.nl_c1z", 0, "^https?://(www\\.)?denieuwebibliotheek\\.nl/", 10, [], [{ e: 1316 }], [{ v: 1316 }], [{ wait: 500 }, { c: 1316 }], [{ timeout: 1e3, check: "none", wv: 1316 }], {}], [1, "auto_NL_discountoffice.nl_2fb", 0, "^https?://(www\\.)?discountoffice\\.nl/", 10, [], [{ e: 1317 }], [{ v: 1317 }], [{ wait: 500 }, { c: 1317 }], [{ timeout: 1e3, check: "none", wv: 1317 }], {}], [1, "auto_NL_ditjesendatjes.nl_0sa", 0, "^https?://(www\\.)?ditjesendatjes\\.nl/", 10, [], [{ e: 1318 }], [{ v: 1318 }], [{ wait: 500 }, { c: 1318 }], [{ timeout: 1e3, check: "none", wv: 1318 }], {}], [1, "auto_NL_duitslandinstituut.nl_d4q", 0, "^https?://(www\\.)?duitslandinstituut\\.nl/", 10, [], [{ e: 1319 }], [{ v: 1319 }], [{ wait: 500 }, { c: 1319 }], [{ timeout: 1e3, check: "none", wv: 1319 }], {}], [1, "auto_NL_effenaar.nl_nia", 0, "^https?://(www\\.)?effenaar\\.nl/", 10, [], [{ e: 1320 }], [{ v: 1320 }], [{ wait: 500 }, { c: 1320 }], [{ timeout: 1e3, check: "none", wv: 1320 }], {}], [1, "auto_NL_eurojackpot.nederlandseloterij.nl_7hr", 0, "^https?://(www\\.)?eurojackpot\\.nederlandseloterij\\.nl/", 10, [], [{ e: 1321 }], [{ v: 1321 }], [{ wait: 500 }, { c: 1321 }], [{ timeout: 1e3, check: "none", wv: 1321 }], {}], [1, "auto_NL_fietsonderdelenoutlet.nl_x8u", 0, "^https?://(www\\.)?fietsonderdelenoutlet\\.nl/", 10, [], [{ e: 1322 }], [{ v: 1322 }], [{ wait: 500 }, { c: 1322 }], [{ timeout: 1e3, check: "none", wv: 1322 }], {}], [1, "auto_NL_followthebeat.nl_mx5", 0, "^https?://(www\\.)?followthebeat\\.nl/", 10, [], [{ e: 1323 }], [{ v: 1323 }], [{ wait: 500 }, { c: 1323 }], [{ timeout: 1e3, check: "none", wv: 1323 }], {}], [1, "auto_NL_frankenergie.nl_9xh", 0, "^https?://(www\\.)?frankenergie\\.nl/", 10, [], [{ e: 1324 }], [{ v: 1324 }], [{ wait: 500 }, { c: 1324 }], [{ timeout: 1e3, check: "none", wv: 1324 }], {}], [1, "auto_NL_gezondheidenwetenschap.be_zgi", 0, "^https?://(www\\.)?gezondheidenwetenschap\\.be/", 10, [], [{ e: 1325 }], [{ v: 1325 }], [{ wait: 500 }, { c: 1325 }], [{ timeout: 1e3, check: "none", wv: 1325 }], {}], [1, "auto_NL_independer.nl_ind", 0, "^https?://(www\\.)?independer\\.nl/", 10, [1326], [{ e: 1327 }], [{ v: 1327 }], [{ c: 1327 }], [{ check: "none", timeout: 2e3, wv: 1326 }], {}], [1, "auto_NL_info.mumc.nl_8s5", 0, "^https?://(www\\.)?info\\.mumc\\.nl/", 10, [], [{ e: 1328 }], [{ v: 1328 }], [{ wait: 500 }, { c: 1328 }], [{ timeout: 1e3, check: "none", wv: 1328 }], {}], [1, "auto_NL_inshared.nl_p70", 0, "^https?://(www\\.)?inshared\\.nl/", 10, [], [{ e: 1329 }], [{ v: 1329 }], [{ wait: 500 }, { c: 1329 }], [{ timeout: 1e3, check: "none", wv: 1329 }], {}], [1, "auto_NL_isvw.nl_7x0", 0, "^https?://(www\\.)?isvw\\.nl/", 10, [], [{ e: 1330 }], [{ v: 1330 }], [{ wait: 500 }, { c: 1330 }], [{ timeout: 1e3, check: "none", wv: 1330 }], {}], [1, "auto_NL_kaartje2go.nl_ecw", 0, "^https?://(www\\.)?kaartje2go\\.nl/", 10, [], [{ e: 1331 }], [{ v: 1331 }], [{ wait: 500 }, { c: 1331 }], [{ timeout: 1e3, check: "none", wv: 1331 }], {}], [1, "auto_NL_kathmandu.nl_7j4", 0, "^https?://(www\\.)?kathmandu\\.nl/", 10, [], [{ e: 1332 }], [{ v: 1332 }], [{ wait: 500 }, { c: 1332 }], [{ timeout: 1e3, check: "none", wv: 1332 }], {}], [1, "auto_NL_kvk.nl_0wn", 0, "^https?://(www\\.)?kvk\\.nl/", 10, [], [{ e: 1333 }], [{ v: 1333 }], [{ wait: 500 }, { c: 1333 }], [{ timeout: 1e3, check: "none", wv: 1333 }], {}], [1, "auto_NL_labplusarts.nl_feg", 0, "^https?://(www\\.)?labplusarts\\.nl/", 10, [], [{ e: 1334 }], [{ v: 1334 }], [{ wait: 500 }, { c: 1334 }], [{ timeout: 1e3, check: "none", wv: 1334 }], {}], [1, "auto_NL_lakenhal.nl_3go", 0, "^https?://(www\\.)?lakenhal\\.nl/", 10, [], [{ e: 1335 }], [{ v: 1335 }], [{ wait: 500 }, { c: 1335 }], [{ timeout: 1e3, check: "none", wv: 1335 }], {}], [1, "auto_NL_lotto.nederlandseloterij.nl_7c8_+1", 0, "^https?://(www\\.)?lotto\\.nederlandseloterij\\.nl/|^https?://(www\\.)?staatsloterij\\.nederlandseloterij\\.nl/", 10, [], [{ e: 1336 }], [{ v: 1336 }], [{ wait: 500 }, { c: 1336 }], [{ timeout: 1e3, check: "none", wv: 1336 }], {}], [1, "auto_NL_magazines-motivatie.nl_6o1", 0, "^https?://(www\\.)?magazines-motivatie\\.nl/", 10, [], [{ e: 1337 }], [{ v: 1337 }], [{ wait: 500 }, { c: 1337 }], [{ timeout: 1e3, check: "none", wv: 1337 }], {}], [1, "auto_NL_makro.nl_ror", 0, "^https?://(www\\.)?makro\\.nl/", 10, [], [{ e: 1338 }], [{ v: 1338 }], [{ wait: 500 }, { c: 1338 }], [{ timeout: 1e3, check: "none", wv: 1338 }], {}], [1, "auto_NL_manufactum.nl_w9l", 0, "^https?://(www\\.)?manufactum\\.nl/", 10, [], [{ e: 1110 }], [{ v: 1110 }], [{ wait: 500 }, { c: 1110 }], [{ timeout: 1e3, check: "none", wv: 1110 }], {}], [1, "auto_NL_matrassencheck.nl_cap", 0, "^https?://(www\\.)?matrassencheck\\.nl/", 10, [], [{ e: 1339 }], [{ v: 1339 }], [{ wait: 500 }, { c: 1339 }], [{ timeout: 1e3, check: "none", wv: 1339 }], {}], [1, "auto_NL_mijn.simyo.nl_xm9", 0, "^https?://(www\\.)?mijn\\.simyo\\.nl/", 10, [], [{ e: 1340 }], [{ v: 1340 }], [{ wait: 500 }, { c: 1340 }], [{ timeout: 1e3, check: "none", wv: 1340 }], {}], [1, "auto_NL_mijngelderland.nl_rkj", 0, "^https?://(www\\.)?mijngelderland\\.nl/", 10, [], [{ e: 1341 }], [{ v: 1341 }], [{ wait: 500 }, { c: 1341 }], [{ timeout: 1e3, check: "none", wv: 1341 }], {}], [1, "auto_NL_milieucentraal.nl_p66", 0, "^https?://(www\\.)?milieucentraal\\.nl/", 10, [], [{ e: 1342 }], [{ v: 1342 }], [{ wait: 500 }, { c: 1342 }], [{ timeout: 1e3, check: "none", wv: 1342 }], {}], [1, "auto_NL_muziekgebouw.nl_cob", 0, "^https?://(www\\.)?muziekgebouw\\.nl/", 10, [], [{ e: 1310 }], [{ v: 1310 }], [{ wait: 500 }, { c: 1310 }], [{ timeout: 1e3, check: "none", wv: 1310 }], {}], [1, "auto_NL_nec-nijmegen.nl_04y", 0, "^https?://(www\\.)?nec-nijmegen\\.nl/", 10, [], [{ e: 1343 }], [{ v: 1343 }], [{ wait: 500 }, { c: 1343 }], [{ timeout: 1e3, check: "none", wv: 1343 }], {}], [1, "auto_NL_nederlandseloterij.nl_b60", 0, "^https?://(www\\.)?nederlandseloterij\\.nl/", 10, [], [{ e: 1344 }], [{ v: 1344 }], [{ wait: 500 }, { c: 1344 }], [{ timeout: 1e3, check: "none", wv: 1344 }], {}], [1, "auto_NL_nl.spankbang.com_bnh_+1", 0, "^https?://(www\\.)?nl\\.spankbang\\.com/|^https?://(www\\.)?spankbang\\.com/", 10, [], [{ e: 1345 }], [{ v: 1345 }], [{ wait: 500 }, { c: 1345 }], [{ timeout: 1e3, check: "none", wv: 1345 }], {}], [1, "auto_NL_orpheus.nl_cxu", 0, "^https?://(www\\.)?orpheus\\.nl/", 10, [], [{ e: 1310 }], [{ v: 1310 }], [{ wait: 500 }, { c: 1310 }], [{ timeout: 1e3, check: "none", wv: 1310 }], {}], [1, "auto_NL_partnerplatform.bol.com_k9h", 0, "^https?://(www\\.)?partnerplatform\\.bol\\.com/", 10, [], [{ e: 1346 }], [{ v: 1346 }], [{ wait: 500 }, { c: 1346 }], [{ timeout: 1e3, check: "none", wv: 1346 }], {}], [1, "auto_NL_persportaal.anp.nl_o32", 0, "^https?://(www\\.)?persportaal\\.anp\\.nl/", 10, [], [{ e: 1347 }], [{ v: 1347 }], [{ wait: 500 }, { c: 1347 }], [{ timeout: 1e3, check: "none", wv: 1347 }], {}], [1, "auto_NL_planteenolijfboom.nl_pdo", 0, "^https?://(www\\.)?planteenolijfboom\\.nl/", 10, [], [{ e: 1348 }], [{ v: 1348 }], [{ wait: 500 }, { c: 1348 }], [{ timeout: 1e3, check: "none", wv: 1348 }], {}], [1, "auto_NL_psv.nl_2pt", 0, "^https?://(www\\.)?psv\\.nl/", 10, [], [{ e: 1349 }], [{ v: 1349 }], [{ wait: 500 }, { c: 1349 }], [{ timeout: 1e3, check: "none", wv: 1349 }], {}], [1, "auto_NL_sanitairkamer.nl_vig", 0, "^https?://(www\\.)?sanitairkamer\\.nl/", 10, [], [{ e: 1350 }], [{ v: 1350 }], [{ wait: 500 }, { c: 1350 }], [{ timeout: 1e3, check: "none", wv: 1350 }], {}], [1, "auto_NL_schaapcitroen.nl_6v0", 0, "^https?://(www\\.)?schaapcitroen\\.nl/", 10, [], [{ e: 1351 }], [{ v: 1351 }], [{ wait: 500 }, { c: 1351 }], [{ timeout: 1e3, check: "none", wv: 1351 }], {}], [1, "auto_NL_scouting.nl_ue4", 0, "^https?://(www\\.)?scouting\\.nl/", 10, [], [{ e: 1352 }], [{ v: 1352 }], [{ wait: 500 }, { c: 1352 }], [{ timeout: 1e3, check: "none", wv: 1352 }], {}], [1, "auto_NL_slachtofferhulp.nl_4m4", 0, "^https?://(www\\.)?slachtofferhulp\\.nl/", 10, [], [{ e: 1353 }], [{ v: 1353 }], [{ wait: 500 }, { c: 1353 }], [{ timeout: 1e3, check: "none", wv: 1353 }], {}], [1, "auto_NL_sprinklr.co_3ww", 0, "^https?://(www\\.)?sprinklr\\.co/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_NL_stadsschouwburg-utrecht.nl_tcz", 0, "^https?://(www\\.)?stadsschouwburg-utrecht\\.nl/", 10, [], [{ e: 1354 }], [{ v: 1354 }], [{ wait: 500 }, { c: 1354 }], [{ timeout: 1e3, check: "none", wv: 1354 }], {}], [1, "auto_NL_stedelijkmuseumschiedam.nl_y6j", 0, "^https?://(www\\.)?stedelijkmuseumschiedam\\.nl/", 10, [], [{ e: 1246 }], [{ v: 1246 }], [{ wait: 500 }, { c: 1246 }], [{ timeout: 1e3, check: "none", wv: 1246 }], {}], [1, "auto_NL_texelsecourant.nl_v4b", 0, "^https?://(www\\.)?texelsecourant\\.nl/", 10, [], [{ e: 1355 }], [{ v: 1355 }], [{ wait: 500 }, { c: 1355 }], [{ timeout: 1e3, check: "none", wv: 1355 }], {}], [1, "auto_NL_ticketswap.com_d3j", 0, "^https?://(www\\.)?ticketswap\\.com/", 10, [], [{ e: 1356 }], [{ v: 1356 }], [{ wait: 500 }, { c: 1356 }], [{ timeout: 1e3, check: "none", wv: 1356 }], {}], [1, "auto_NL_uit.inapeldoorn.nl_gt5", 0, "^https?://(www\\.)?uit\\.inapeldoorn\\.nl/", 10, [], [{ e: 1357 }], [{ v: 1357 }], [{ wait: 500 }, { c: 1357 }], [{ timeout: 1e3, check: "none", wv: 1357 }], {}], [1, "auto_NL_vlaanderen.be_c8w", 0, "^https?://(www\\.)?vlaanderen\\.be/", 10, [], [{ e: 1358 }], [{ v: 1358 }], [{ wait: 500 }, { c: 1358 }], [{ timeout: 1e3, check: "none", wv: 1358 }], {}], [1, "auto_NL_vvaa.nl_avs", 0, "^https?://(www\\.)?vvaa\\.nl/", 10, [], [{ e: 1359 }], [{ v: 1359 }], [{ wait: 500 }, { c: 1359 }], [{ timeout: 1e3, check: "none", wv: 1359 }], {}], [1, "auto_NL_werkenbijdefensie.nl_0cv", 0, "^https?://(www\\.)?werkenbijdefensie\\.nl/", 10, [], [{ e: 1360 }], [{ v: 1360 }], [{ wait: 500 }, { c: 1360 }], [{ timeout: 1e3, check: "none", wv: 1360 }], {}], [1, "auto_NL_winkelstraat.nl_hxj", 0, "^https?://(www\\.)?winkelstraat\\.nl/", 10, [], [{ e: 1361 }], [{ v: 1361 }], [{ wait: 500 }, { c: 1361 }], [{ timeout: 1e3, check: "none", wv: 1361 }], {}], [1, "auto_NL_woonnetrijnmond.nl_itj", 0, "^https?://(www\\.)?woonnetrijnmond\\.nl/", 10, [], [{ e: 1362 }], [{ v: 1362 }], [{ wait: 500 }, { c: 1362 }], [{ timeout: 1e3, check: "none", wv: 1362 }], {}], [1, "auto_NL_zuidas.nl_abm", 0, "^https?://(www\\.)?zuidas\\.nl/", 10, [], [{ e: 1363 }], [{ v: 1363 }], [{ wait: 500 }, { c: 1363 }], [{ timeout: 1e3, check: "none", wv: 1363 }], {}], [1, "auto_NO_apxml.com_l6n", 0, "^https?://(www\\.)?apxml\\.com/", 10, [], [{ e: 1364 }], [{ v: 1364 }], [{ wait: 500 }, { c: 1364 }], [{ timeout: 1e3, check: "none", wv: 1364 }], {}], [1, "auto_NO_arbetsformedlingen.se_wxz", 0, "^https?://(www\\.)?arbetsformedlingen\\.se/", 10, [], [{ e: 1365 }], [{ v: 1365 }], [{ wait: 500 }, { c: 1365 }], [{ timeout: 1e3, check: "none", wv: 1365 }], {}], [1, "auto_NO_arronet.se_vo5", 0, "^https?://(www\\.)?arronet\\.se/", 10, [], [{ e: 1366 }], [{ v: 1366 }], [{ wait: 500 }, { c: 1366 }], [{ timeout: 1e3, check: "none", wv: 1366 }], {}], [1, "auto_NO_avanza.se_b5b", 0, "^https?://(www\\.)?avanza\\.se/", 10, [], [{ e: 1367 }], [{ v: 1367 }], [{ wait: 500 }, { c: 1367 }], [{ timeout: 1e3, check: "none", wv: 1367 }], {}], [1, "auto_NO_avidafinance.com_jwc", 0, "^https?://(www\\.)?avidafinance\\.com/", 10, [], [{ e: 1368 }], [{ v: 1368 }], [{ wait: 500 }, { c: 1368 }], [{ timeout: 1e3, check: "none", wv: 1368 }], {}], [1, "auto_NO_beducated.com_jjt", 0, "^https?://(www\\.)?beducated\\.com/", 10, [], [{ e: 1369 }], [{ v: 1369 }], [{ wait: 500 }, { c: 1369 }], [{ timeout: 1e3, check: "none", wv: 1369 }], {}], [1, "auto_NO_dsa.no_w2b", 0, "^https?://(www\\.)?dsa\\.no/", 10, [], [{ e: 1370 }], [{ v: 1370 }], [{ wait: 500 }, { c: 1370 }], [{ timeout: 1e3, check: "none", wv: 1370 }], {}], [1, "auto_NO_goteborg.com_q7v", 0, "^https?://(www\\.)?goteborg\\.com/", 10, [], [{ e: 1371 }], [{ v: 1371 }], [{ wait: 500 }, { c: 1371 }], [{ timeout: 1e3, check: "none", wv: 1371 }], {}], [1, "auto_NO_hacksmith.store_49t", 0, "^https?://(www\\.)?hacksmith\\.store/", 10, [], [{ e: 1372 }], [{ v: 1372 }], [{ wait: 500 }, { c: 1372 }], [{ timeout: 1e3, check: "none", wv: 1372 }], {}], [1, "auto_NO_hotelfinse1222.no_zpm", 0, "^https?://(www\\.)?hotelfinse1222\\.no/", 10, [], [{ e: 1348 }], [{ v: 1348 }], [{ wait: 500 }, { c: 1348 }], [{ timeout: 1e3, check: "none", wv: 1348 }], {}], [1, "auto_NO_jaksta.com_own", 0, "^https?://(www\\.)?jaksta\\.com/", 10, [], [{ e: 1373 }], [{ v: 1373 }], [{ wait: 500 }, { c: 1373 }], [{ timeout: 1e3, check: "none", wv: 1373 }], {}], [1, "auto_NO_ledigajobb.se_tx3", 0, "^https?://(www\\.)?ledigajobb\\.se/", 10, [], [{ e: 1374 }], [{ v: 1374 }], [{ wait: 500 }, { c: 1374 }], [{ timeout: 1e3, check: "none", wv: 1374 }], {}], [1, "auto_NO_liseberg.se_qy4", 0, "^https?://(www\\.)?liseberg\\.se/", 10, [], [{ e: 1375 }], [{ v: 1375 }], [{ wait: 500 }, { c: 1375 }], [{ timeout: 1e3, check: "none", wv: 1375 }], {}], [1, "auto_NO_nordnet.se_2en", 0, "^https?://(www\\.)?nordnet\\.se/", 10, [], [{ e: 1376 }], [{ v: 1376 }], [{ wait: 500 }, { c: 1376 }], [{ timeout: 1e3, check: "none", wv: 1376 }], {}], [1, "auto_NO_pcsx2.net_grl", 0, "^https?://(www\\.)?pcsx2\\.net/", 10, [], [{ e: 1377 }], [{ v: 1377 }], [{ wait: 500 }, { c: 1377 }], [{ timeout: 1e3, check: "none", wv: 1377 }], {}], [1, "auto_NO_regeringen.se_uhv", 0, "^https?://(www\\.)?regeringen\\.se/", 10, [], [{ e: 1378 }], [{ v: 1378 }], [{ wait: 500 }, { c: 1378 }], [{ timeout: 1e3, check: "none", wv: 1378 }], {}], [1, "auto_NO_saseurobonusmastercard.se_oqd", 0, "^https?://(www\\.)?saseurobonusmastercard\\.se/", 10, [], [{ e: 1379 }], [{ v: 1379 }], [{ wait: 500 }, { c: 1379 }], [{ timeout: 1e3, check: "none", wv: 1379 }], {}], [1, "auto_NO_sverigesradio.se_a00", 0, "^https?://(www\\.)?sverigesradio\\.se/", 10, [], [{ e: 1380 }], [{ v: 1380 }], [{ wait: 500 }, { c: 1380 }], [{ timeout: 1e3, check: "none", wv: 1380 }], {}], [1, "auto_NO_svtplay.se_ujn", 0, "^https?://(www\\.)?svtplay\\.se/", 10, [], [{ e: 1381 }], [{ v: 1381 }], [{ wait: 500 }, { c: 1381 }], [{ timeout: 1e3, check: "none", wv: 1381 }], {}], [1, "auto_NO_swedbank.se_63w", 0, "^https?://(www\\.)?swedbank\\.se/", 10, [], [{ e: 1382 }], [{ v: 1382 }], [{ wait: 500 }, { c: 1382 }], [{ timeout: 1e3, check: "none", wv: 1382 }], {}], [1, "auto_NO_tingstad.com_z23", 0, "^https?://(www\\.)?tingstad\\.com/", 10, [], [{ e: 1383 }], [{ v: 1383 }], [{ wait: 500 }, { c: 1383 }], [{ timeout: 1e3, check: "none", wv: 1383 }], {}], [1, "auto_NO_truecaller.com_sdg", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 1384 }], [{ v: 1384 }], [{ wait: 500 }, { c: 1384 }], [{ timeout: 1e3, check: "none", wv: 1384 }], {}], [1, "auto_NO_uu.se_eea", 0, "^https?://(www\\.)?uu\\.se/", 10, [], [{ e: 1385 }], [{ v: 1385 }], [{ wait: 500 }, { c: 1385 }], [{ timeout: 1e3, check: "none", wv: 1385 }], {}], [1, "auto_NO_webank.it_6rx", 0, "^https?://(www\\.)?webank\\.it/", 10, [], [{ e: 1386 }], [{ v: 1386 }], [{ wait: 500 }, { c: 1386 }], [{ timeout: 1e3, check: "none", wv: 1386 }], {}], [1, "auto_NO_xrealgirl.com_r42", 0, "^https?://(www\\.)?xrealgirl\\.com/", 10, [], [{ e: 1387 }], [{ v: 1387 }], [{ wait: 500 }, { c: 1387 }], [{ timeout: 1e3, check: "none", wv: 1387 }], {}], [1, "auto_US_amazon.jobs_0", 0, "^https?://(www\\.)?amazon\\.jobs/", 10, [], [{ e: 1388 }], [{ v: 1388 }], [{ wait: 500 }, { c: 1388 }], [{ timeout: 1e3, check: "none", wv: 1388 }], {}], [1, "auto_US_amsoil.com_fgr", 0, "^https?://(www\\.)?amsoil\\.com/", 10, [], [{ e: 1389 }], [{ v: 1389 }], [{ c: 1389 }], [], {}], [1, "auto_US_balsamhill.com_oxq", 0, "^https?://(www\\.)?balsamhill\\.com/", 10, [], [{ e: 1390 }], [{ v: 1390 }], [{ wait: 500 }, { c: 1390 }], [{ timeout: 1e3, check: "none", wv: 1390 }], {}], [1, "auto_US_computerworld.com_0_+1", 0, "^https?://(www\\.)?computerworld\\.com/|^https?://(www\\.)?csoonline\\.com/", 10, [], [{ e: 1391 }], [{ v: 1391 }], [{ text: "Do not accept", c: 1391 }], [], {}], [1, "auto_US_coupons.slickdeals.net_wya", 0, "^https?://(www\\.)?coupons\\.slickdeals\\.net/", 10, [], [{ e: 1392 }], [{ v: 1392 }], [{ wait: 500 }, { c: 1392 }], [{ timeout: 1e3, check: "none", wv: 1392 }], {}], [1, "auto_US_deezer.com_0", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 1393 }], [{ v: 1393 }], [{ text: "Refuse", c: 1393 }], [], {}], [1, "auto_US_emagine-entertainment.com_0", 0, "^https?://(www\\.)?emagine-entertainment\\.com/", 10, [], [{ e: 1394 }], [{ v: 1394 }], [{ wait: 500 }, { c: 1394 }], [{ timeout: 1e3, check: "none", wv: 1394 }], {}], [1, "auto_US_fawesome.tv_gjx", 0, "^https?://(www\\.)?fawesome\\.tv/", 10, [], [{ e: 1395 }], [{ v: 1395 }], [{ wait: 500 }, { c: 1395 }], [{ timeout: 1e3, check: "none", wv: 1395 }], {}], [1, "auto_US_forum.affinity.serif.com_0", 0, "^https?://(www\\.)?forum\\.affinity\\.serif\\.com/", 10, [], [{ e: 1396 }], [{ v: 1396 }], [{ text: "\xA0Reject Cookies", c: 1396 }], [], {}], [1, "auto_US_forum.prusa3d.com_0", 0, "^https?://(www\\.)?forum\\.prusa3d\\.com/", 10, [], [{ e: 1397 }], [{ v: 1397 }], [{ text: "Reject All", c: 1397 }], [], {}], [1, "auto_US_greenpan.us_cko", 0, "^https?://(www\\.)?greenpan\\.us/", 10, [], [{ e: 1040 }], [{ v: 1040 }], [{ wait: 500 }, { c: 1040 }], [{ timeout: 1e3, check: "none", wv: 1040 }], {}], [1, "auto_US_interactivebrokers.com_0", 0, "^https?://(www\\.)?interactivebrokers\\.com/", 10, [], [{ e: 1398 }], [{ v: 1398 }], [{ text: "Reject All Cookies", c: 1398 }], [], {}], [1, "auto_US_ligonier.org_0", 0, "^https?://(www\\.)?ligonier\\.org/", 10, [], [{ e: 1399 }], [{ v: 1399 }], [{ text: "Strictly Necessary", c: 1399 }], [], {}], [1, "auto_US_lilly.com_lh6_+1", 0, "^https?://(www\\.)?lilly\\.com/|^https?://(www\\.)?zepbound\\.lilly\\.com/", 10, [], [{ e: 1400 }], [{ v: 1400 }], [{ wait: 500 }, { c: 1400 }], [{ timeout: 1e3, check: "none", wv: 1400 }], {}], [1, "auto_US_mixedbread.ai_tge", 0, "^https?://(www\\.)?mixedbread\\.com/", 10, [], [{ e: 1364 }], [{ v: 1364 }], [{ wait: 500 }, { c: 1364 }], [{ timeout: 1e3, check: "none", wv: 1364 }], {}], [1, "auto_US_musicnotes.com_0", 0, "^https?://(www\\.)?musicnotes\\.com/", 10, [], [{ exists: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }], [{ visible: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }], [{ if: { exists: ["#polaris-css-lockdown-container", "[data-testid='oneClickOptOutLink']"] }, then: [{ waitForThenClick: ["#polaris-css-lockdown-container", "[data-testid='oneClickOptOutLink']"] }], else: [{ hide: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }] }], [], {}], [1, "auto_US_newmedicare.com_6jp", 0, "^https?://(www\\.)?newmedicare\\.com/", 10, [], [{ e: 1401 }], [{ v: 1401 }], [{ wait: 500 }, { c: 1401 }], [{ timeout: 1e3, check: "none", wv: 1401 }], {}], [1, "auto_US_newsbreak.com_0", 0, "^https?://(www\\.)?newsbreak\\.com/", 10, [], [{ e: 1402 }], [{ v: 1402 }], [{ c: 1402 }], [], {}], [1, "auto_US_peptidesciences.com_0", 0, "^https?://(www\\.)?peptidesciences\\.com/", 10, [], [{ e: 1403 }], [{ v: 1403 }], [{ wait: 500 }, { c: 1403 }], [{ timeout: 1e3, check: "none", wv: 1403 }], {}], [1, "auto_US_semrush.com_0", 0, "^https?://(www\\.)?semrush\\.com/", 10, [], [{ e: 1404 }], [{ v: 1404 }], [{ wait: 500 }, { c: 1404 }], [{ timeout: 1e3, check: "none", wv: 1404 }], {}], [1, "auto_US_sso.passport.yandex.ru_0_+5", 0, "^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?tv\\.yandex\\.com/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?yandex\\.com/", 10, [], [{ e: 1405 }], [{ v: 1405 }], [{ text: "Allow essential cookies", c: 1405 }], [], {}], [1, "auto_US_truecaller.com_0", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 1406 }], [{ v: 1406 }], [{ text: "Accept Necessary Cookies", c: 1406 }], [], {}], [1, "auto_US_weingartz.com_aso", 0, "^https?://(www\\.)?weingartz\\.com/", 10, [], [{ e: 1407 }], [{ v: 1407 }], [{ c: 1407 }], [], {}], [1, "aytm", 2, "^https?://(www\\.|)?aytm\\.com/", 22, [1408], [{ e: 1409 }], [{ v: 1409 }], [{ c: 1409 }], [], {}], [1, "bahn-de", 0, "^https://(www\\.)?bahn\\.de/", 10, [], [{ exists: ["body > div:first-child", "#consent-layer"] }], [{ visible: ["body > div:first-child", "#consent-layer"] }], [{ waitForThenClick: ["body > div:first-child", "#consent-layer .js-accept-essential-cookies"] }], [{ eval: "EVAL_BAHN_TEST" }], { intermediate: false }], [1, "bandcamp.com", 2, "^https://([a-z0-9-]+\\.)?bandcamp\\.com", 22, [], [{ e: 1410 }], [{ v: 1411 }], [{ c: 1412 }], [{ cc: 1413 }], {}], [1, "bankmillennium.pl", 2, "^https?://(www\\.)?bankmillennium\\.pl/", 22, [1414], [{ e: 1414 }], [{ v: 1414 }], [{ waitForThenClick: ["#cookie_modal_placeholder dialog.bm-modal--cookie", "xpath///button[contains(normalize-space(.), 'Odrzu')]"] }], [{ cc: 1415 }, { cc: 1416 }], {}], [1, "bbc.com", 2, "^https://(www\\.)?bbc\\.(com|co\\.uk)/", 22, [1417, 1418], [{ any: [{ e: 1419 }, { e: 1418 }] }], [{ any: [{ v: 1419 }, { v: 1418 }] }], [{ if: { e: 1418 }, then: [{ c: 1420 }], else: [{ c: 1421 }] }], [{ cc: 1422 }], {}], [1, "bcferries.com", 2, "^https?://(\\w+\\.)?bcferries\\.com/", 22, [1423], [{ e: 1424 }], [{ v: 1423 }], [{ c: 1425 }, { c: 1426 }, { c: 1427 }], [], {}], [1, "bibliotheek-nl", 0, "^https?://(?:www\\.)?(?:onlinebibliotheek|bibliotheek)\\.nl/", 10, [1428], [{ e: 1429 }], [{ v: 1428 }], [{ c: 1430 }], [], {}], [1, "canyon.com", 2, "^https://www\\.canyon\\.com/", 22, [1431], [{ e: 1431 }], [{ v: 1431 }], [{ k: 1432 }, { c: 1433 }], [], {}], [1, "carre.nl", 2, "^https?://(www\\.)?carre\\.nl/", 10, [1434], [{ e: 1435 }], [{ v: 1434 }], [{ c: 1435 }], [], {}], [1, "ccm-net", 1, "^https?://([\\w-]+\\.)?ccm\\.net/", 22, [30, 1436], [{ any: [{ e: 31 }, { e: 1436 }] }], [{ any: [{ v: 30 }, { v: 1436 }] }], [{ h: 1436 }, { if: { v: 30 }, then: [{ waitForThenClick: ["iframe[srcdoc*='frame-root']", ".button__skip"] }] }], [], {}], [1, "channel4.com", 2, "^https?://(www\\.)?channel4\\.com/", 10, [667], [{ e: 667 }], [{ v: 1437 }], [{ c: 1438 }], [{ cc: 1439 }], {}], [1, "chatgpt", 0, "^https?://(\\w+\\.)?chatgpt\\.com/", 10, [1440], [{ e: 1441 }], [{ v: 1440 }], [{ c: 1442 }], [{ timeout: 1e3, check: "none", wv: 1443 }], {}], [1, "clustrmaps.com", 1, "^https://(www\\.)?clustrmaps\\.com/", 22, [1444], [{ e: 1444 }], [{ v: 1444 }], [{ h: 1444 }], [], {}], [1, "copilot-microsoft", 0, "^https://copilot\\.microsoft\\.com/", 10, [9], [{ e: 9 }], [{ v: 9 }], [{ c: 1445 }], [{ cc: 1446 }], {}], [1, "csu-landtag-de", 2, "^https://(www\\.|)?csu-landtag\\.de", 22, [1447], [{ e: 1447 }], [{ v: 1447 }], [{ k: 1448 }], [], {}], [1, "ctv-disneyonice", 2, "^https://www\\.ctv\\.co\\.jp/disneyonice(?:/|[?#]|$)", 22, [1449], [{ e: 1450 }], [{ v: 1450 }], [{ c: 1451 }], [{ cc: 1452 }], {}], [1, "dailymotion.com", 2, "^https://(www\\.)?dailymotion\\.com/", 22, [1453], [{ e: 1454 }], [{ v: 1455 }], [{ c: 1456 }], [{ cc: 1457 }], {}], [1, "deliveroo", 2, "^https?://(\\w+\\.)?deliveroo\\.(co\\.uk|\\w+)/", 22, [1458], [{ e: 1458 }], [{ v: 1458 }], [{ c: 1459 }, { check: "none", wv: 1458 }], [{ cc: 1460 }], {}], [1, "delta.com", 1, "^https://www\\.delta\\.com/", 22, [1461], [{ e: 1462 }], [{ v: 1462 }], [{ h: 1462 }], [], {}], [1, "depop", 0, "^https://(www\\.)?depop\\.com/", 22, [1463], [{ e: 1464 }], [{ v: 1464 }], [{ c: 1465 }, { all: true, optional: true, k: 1466 }, { c: 1467 }], [{ cc: 1468 }], {}], [1, "dji", 0, "^https://(\\w+\\.)+dji\\.com/", 22, [1469], [{ e: 1470 }], [{ e: 1469 }], [{ c: 1471 }], [{ cc: 1472 }], {}], [1, "dndbeyond", 2, "^https://(www\\.)?dndbeyond\\.com/", 22, [1473], [{ e: 1473 }], [{ v: 1473 }], [{ c: 1474 }], [{ cc: 1475 }], {}], [1, "ebay", 0, "^https://(www\\.)?ebay\\.([.a-z]+)/", 10, [1476], [{ e: 1476 }], [{ v: 1476 }], [{ c: 1477 }, { check: "none", timeout: 2e3, optional: true, wv: 1476 }, { if: { v: 1476 }, then: [{ timeout: 2e3, c: 1477 }] }], [], {}], [1, "ecosia", 2, "^https://www\\.ecosia\\.org/", 22, [1478], [{ e: 1479 }], [{ v: 1479 }], [{ c: 1480 }], [], {}], [1, "edpb-edps", 2, "^https://(www\\.)?(edpb|edps)\\.europa\\.eu/", 22, [1481], [{ e: 1482 }], [{ v: 1481 }], [{ c: 1482 }], [{ cc: 1483 }], {}], [1, "ef-ccpa", 0, "^https://(www\\.)?eforms\\.com", 22, [1484], [{ e: 1484 }], [{ v: 1484 }], [{ c: 1485 }], [], {}], [1, "eltax-lta-go-jp", 2, "^https://(?:[^/]+\\.)?eltax\\.lta\\.go\\.jp/", 22, [1486], [{ e: 1487 }], [{ v: 1487 }], [{ c: 1487 }], [], {}], [1, "epidemicsound.com", 2, "^https?://(\\w+\\.)?epidemicsound\\.com/", 22, [1488], [{ e: 1489 }], [{ v: 1490 }], [{ w: 1491 }, { c: 1490 }, { wv: 1492 }, { all: true, optional: true, k: 1493 }, { c: 1491 }, { check: "none", wv: 1490 }], [{ cc: 1460 }], {}], [1, "escaparium.ca", 2, "^https://(www\\.)?escaparium\\.ca/", 22, [], [{ e: 1494 }], [{ v: 1495 }], [{ c: 1496 }, { c: 1497 }], [{ cc: 1498 }], {}], [1, "espace-personnel.agirc-arrco.fr", 2, "^https://espace-personnel\\.agirc-arrco\\.fr/", 22, [1499], [{ e: 1500 }], [{ v: 1500 }], [{ c: 1501 }], [], {}], [1, "europa-eu", 2, "^https://([a-z\\.]*\\.)?europa\\.eu/", 22, [667], [{ e: 1502 }], [{ v: 1502 }], [{ c: 1503, h: 1502 }], [], {}], [1, "facebook", 2, "^https://(www\\.)?facebook\\.com/", 22, [], [{ e: 1504 }], [{ v: 1504 }], [{ c: 1504 }, { check: "none", wv: 1504 }], [], {}], [1, "facebook-mobile", 2, "^https://(m|mbasic)\\.facebook\\.com/", 22, [1505], [{ e: 1505 }], [{ v: 1505 }], [{ c: 1506 }, { check: "none", wv: 1505 }], [], {}], [1, "financestrategists.com", 1, "^https://(www\\.)?financestrategists\\.com/", 22, [1507], [{ e: 1507 }], [{ v: 1507 }], [{ h: 1507 }], [], {}], [1, "geeks-for-geeks", 1, "^https://www\\.geeksforgeeks\\.org/", 22, [1508], [{ e: 1508 }], [{ v: 1508 }], [{ h: 1508 }], [], {}], [1, "geni.com", 1, "^https://(www\\.)?geni\\.com/", 22, [1509], [{ e: 1509 }], [{ v: 1509 }], [{ h: 1509 }], [], {}], [1, "glastonburyfestivals", 0, "^https://(\\w+\\.)?glastonburyfestivals\\.co\\.uk/", 22, [1510], [{ e: 1511 }], [{ v: 1511 }], [{ c: 1512 }], [], {}], [1, "groundnews", 0, "^https://(www\\.)?ground\\.news/", 22, [], [{ e: 1513 }], [{ v: 1513 }], [{ waitForThenClick: [".fixed:has([data-testid=closeCookieBanner])", "xpath///button[contains(., 'Manage cookies')]"] }, { waitFor: ["[data-testid=modal]", "xpath///span[contains(., 'Essential cookies')]"] }, { click: ["[data-testid=modal]", "xpath///button[contains(., 'On')]"], all: true, optional: true }, { waitForThenClick: ["[data-testid=modal]", "xpath///button[contains(., 'Save & reload')]"] }], [], {}], [1, "hashicorp", 2, "^https://[a-z]*\\.hashicorp\\.com/", 22, [1514], [{ e: 1514 }], [{ v: 1514 }], [{ c: 1515 }, { c: 1516 }], [], {}], [1, "hearst-us", 1, "^https://(www\\.)?(menshealth|womenshealthmag|cosmopolitan|esquire|elle|elledecor|harpersbazaar|goodhousekeeping|popularmechanics|caranddriver|delish|veranda|roadandtrack|bestproducts|countryliving|housebeautiful|bicycling|biography|pitchfork|prevention|runnersworld|thepioneerwoman|townandcountrymag)\\.com/", 10, [1517], [{ e: 1517 }], [{ check: "any", v: 1517 }], [{ h: 1517 }], [{ check: "none", v: 1517 }], {}], [1, "hetzner.com", 2, "^https://www\\.hetzner\\.com/", 22, [1518], [{ e: 1518 }], [{ v: 1518 }], [{ k: 1519 }], [], {}], [1, "ibanez", 2, "^https?://www\\.ibanez\\.com/", 22, [1520], [{ e: 1521 }], [{ v: 1522 }], [{ c: 1521 }], [{ cc: 1523 }], {}], [1, "imdb", 0, "^https://(www\\.)?(m\\.)?imdb.com/", 22, [1514], [{ e: 1514 }], [{ v: 1514 }], [{ c: 1524 }], [], {}], [1, "instagram", 2, "^https://www\\.instagram\\.com/", 22, [], [{ e: 1525 }], [{ v: 1525 }], [{ c: 1526 }, { wait: 2e3 }], [], {}], [1, "interia", 2, "^https://(www\\.)?interia\\.pl/", 22, [1527], [{ e: 1527 }], [{ v: 1527 }], [{ c: 1528 }, { c: 1529 }], [{ check: "none", timeout: 3e3, wv: 1527 }], {}], [1, "itopvpn.com", 1, "^https://(www\\.)?itopvpn.com/", 22, [], [{ e: 1530 }], [{ e: 1530 }], [{ h: 1530 }], [], {}], [1, "justgiving.com", 2, "^https://(?:[a-z0-9-]+\\.)?justgiving\\.com/", 22, [1531], [{ any: [{ e: 1531 }, { e: 1532 }] }], [{ v: 1531 }, { e: 1533 }], [{ c: 1533 }], [{ wait: 500 }, { cc: 1534 }], {}], [1, "kickstarter.com", 2, "^https?://(www\\.)?kickstarter\\.com/", 22, [1535], [{ e: 1536 }], [{ v: 1536 }], [{ c: 1537 }], [{ check: "none", v: 1536 }], {}], [1, "kingrecords-co-jp", 2, "^https?://(?:[^/]+\\.)?kingrecords\\.co\\.jp/", 22, [1538], [{ e: 1539 }], [{ v: 1539 }], [{ timeout: 1e4, optional: true, w: 1540 }, { c: 1539 }, { if: { v: 1541 }, then: [{ wait: 1e3 }, { k: 1539 }] }, { check: "none", timeout: 5e3, wv: 1541 }], [{ cc: 1542 }], {}], [1, "kleinanzeigen-de", 2, "^https?://(www\\.)?kleinanzeigen\\.de", 22, [1543], [{ e: 1544 }], [{ v: 1544 }], [{ k: 1544 }], [], {}], [1, "leafly", 1, "^https://(www\\.)?leafly\\.com/", 22, [], [{ e: 1545 }], [{ v: 1545 }], [{ h: 1545 }], [], {}], [1, "lendable", 2, "^https?://([\\w-]+\\.)?lendable\\.co\\.uk/", 22, [1546], [{ e: 1547 }], [{ v: 1546 }], [{ c: 1548 }, { c: 1549 }, { check: "none", wv: 1546 }], [{ cc: 1550 }], {}], [1, "lucozade.com", 2, "^https://(www\\.)?lucozade\\.com/", 22, [1551, 1552], [{ e: 1551 }], [{ v: 1551 }], [{ c: 1553 }], [{ cc: 1554 }], {}], [1, "massgeneralbrigham", 0, "^https?://(\\w+\\.)?massgeneralbrigham\\.org/", 22, [1555], [{ e: 1556 }], [{ v: 1556 }], [{ c: 1556 }], [{ negated: true, e: 1555 }], {}], [1, "maticrobots-com", 2, "^https?://(\\w+\\.)?maticrobots\\.com/", 22, [1557], [{ e: 1557 }], [{ v: 1557 }], [{ c: 1558 }, { wv: 1559 }, { c: 1560 }, { c: 1561 }], [], {}], [1, "medium", 1, "^https://([a-z0-9-]+\\.)?medium\\.com/", 10, [], [{ e: 1562 }], [{ v: 1562 }], [{ h: 1562 }], [], {}], [1, "midway-usa", 1, "^https://www\\.midwayusa\\.com/", 22, [1563], [{ exists: ['div[aria-label="Cookie Policy Banner"]'] }], [{ v: 1563 }], [{ h: 1564 }], [], {}], [1, "miles-and-more", 2, "^https://(www\\.)?miles-and-more\\.com/", 22, [1565], [{ e: 1566 }], [{ v: 1567 }], [{ wait: 500 }, { c: 1567 }], [{ cc: 1568 }], {}], [1, "mipro.com.tw", 2, "^https?://(\\w+\\.)?mipro\\.com\\.tw/", 22, [1569], [{ e: 1570 }], [{ v: 1571 }], [{ c: 1571 }], [{ negated: true, e: 1569 }], {}], [1, "msn", 0, "^https?://(www\\.)?msn\\.com/", 10, [1572, 1573], [{ e: 1572 }], [{ v: 1572 }], [{ c: 1574 }], [{ cc: 1460 }], {}], [1, "nba.com", 1, "^https://(www\\.)?nba\\.com/", 22, [1575], [{ e: 1575 }], [{ v: 1575 }], [{ h: 1575 }], [], {}], [1, "netbeat.de", 2, "^https://(www\\.)?netbeat\\.de/", 22, [1576], [{ e: 1576 }], [{ v: 1576 }], [{ c: 1577 }], [], {}], [1, "nhnieuws", 2, "^https://(www\\.)?nhnieuws\\.nl/", 22, [1578], [{ e: 1579 }], [{ check: "any", v: 1579 }], [{ c: 1580 }], [{ eval: "EVAL_NHNIEUWS_TEST" }], {}], [1, "nike", 2, "^https://(www\\.)?nike\\.com/", 22, [], [{ e: 1581 }], [{ v: 1581 }], [{ all: true, c: 1582 }, { c: 1583 }], [], {}], [1, "nos.nl", 1, "^https://nos\\.nl/", 22, [1584], [{ e: 1584 }], [{ visible: ["ccm-notification"] }], [{ h: 1584 }], [], {}], [1, "nutritionix.com", 0, "^https://(www\\.)?nutritionix\\.com/", 22, [1585], [{ e: 1585 }], [{ v: 1585 }], [{ waitForThenClick: ["gdpr-banner", "xpath///button[contains(., 'Refuse')]"] }], [], {}], [1, "ok", 0, "^https://ok\\.ru/", 22, [1586], [{ e: 1587 }], [{ v: 1587 }], [{ w: 1588 }, { wait: 1e3 }, { k: 1588 }, { wv: 1589 }, { wait: 500 }, { all: true, optional: true, k: 1590 }, { c: 1591 }, { check: "none", wv: 1589 }], [], {}], [1, "onlyFans.com", 2, "^https://onlyfans\\.com/", 22, [1592], [{ e: 1592 }], [{ e: 1592 }], [{ k: 1593 }, { if: { e: 1594 }, then: [{ all: true, k: 1595 }, { k: 1596 }] }], [], {}], [1, "openai", 0, "^https://([a-z0-9-]+\\.)?openai\\.com/", 22, [1597], [{ e: 1597 }], [{ v: 1597 }], [{ c: 1598 }], [{ wait: 500 }, { cc: 1599 }], {}], [1, "opera.com", 0, "^https?://(www\\.|)?opera\\.com/", 22, [1408], [{ e: 1600 }], [{ v: 1600 }], [{ all: true, timeout: 500, optional: true, c: 1601 }, { c: 1602 }], [{ cc: 1603 }, { negated: true, cc: 1604 }], {}], [1, "ourworldindata", 2, "^https://ourworldindata\\.org/", 22, [1605], [{ e: 1605 }], [{ v: 1606 }], [{ c: 1607 }], [], {}], [1, "paychex", 1, "^https://(www\\.)?paychex\\.com/", 22, [1608], [{ e: 1608 }], [{ v: 1608 }], [{ h: 1608 }], [], {}], [1, "paypal-cookieprefs", 2, "^https://www\\.paypal\\.com/myaccount/privacy/cookiePrefs\\?", 22, [1609], [{ e: 1610 }], [{ v: 1610 }], [{ all: true, optional: true, k: 1611 }, { timeout: 15e3, c: 1612 }], [{ wait: 1e3 }, { cc: 641 }], {}], [1, "pinterest-business", 2, "^https://[a-z]*\\.pinterest\\.com/", 22, [1613], [{ e: 1613 }], [{ v: 1614 }], [{ c: 1615 }], [], {}], [1, "plos", 0, "^https://([.a-zA-Z0-9-]+\\.)?plos\\.org/", 22, [1408], [{ e: 1616 }], [{ v: 1616 }], [{ all: true, optional: true, k: 1617 }, { waitForThenClick: ["#cookie-consent", "xpath///button[contains(., 'Save Selected')]"] }], [{ cc: 1618 }], {}], [1, "pornhub-compact-cookie-banner", 0, "^https://(www\\.)?pornhub\\.com/", 22, [1619], [{ e: 1620 }], [{ v: 1620 }], [{ k: 1620 }], [], {}], [1, "pornhub.com", 0, "^https://(www\\.)?pornhub\\.com/", 22, [1621], [{ e: 1622 }], [{ v: 1622 }], [{ if: { e: 1623 }, then: [{ c: 1623 }] }, { if: { e: 1624 }, then: [{ k: 1624 }], else: [{ c: 1625 }] }], [], {}], [1, "pornpics.de", 2, "^https?://(www\\.)?pornpics\\.(com|de)/", 22, [], [{ e: 1626 }], [{ v: 1626 }], [{ c: 1627 }], [{ cc: 1628 }], {}], [2, "postnl", 0, "^https://([a-z]*\\.)?postnl\\.nl/", 22, [1629], [{ e: 1629 }], [{ visible: ["pnl-cookie-wall-widget", ".pnl-cookie-wall"] }], [{ waitForThenClick: ["pnl-cookie-wall-widget", ".cookie-overview__footer button.stamp-button--variant-secondary:last-child"] }, { waitForVisible: ["pnl-cookie-wall-widget", ".pnl-cookie-wall"], check: "none", timeout: 3e3, optional: true }, { setStyle: "", selector: "body", optional: true }], [{ cc: 1630 }], {}], [1, "povr", 0, "^https://povr\\.com/", 22, [], [{ e: 1631 }], [{ v: 1631 }], [{ h: 1632 }, { if: { e: 1633 }, then: [{ w: 1634 }, { all: true, optional: true, k: 1635 }, { c: 1636 }, { eval: "EVAL_POVR_GOBACK" }], else: [{ c: 1637 }] }], [], {}], [1, "productz.com", 2, "^https://productz\\.com/", 22, [], [{ e: 1638 }], [{ v: 1638 }], [{ c: 1639 }], [], {}], [1, "raspberrypi.com", 0, "^https://([a-z0-9-]+\\.)?raspberrypi\\.com/", 10, [], [{ e: 1640 }], [{ v: 1640 }], [{ c: 1640 }], [{ check: "none", v: 1640 }], {}], [1, "readly", 2, "^https://([a-z0-9-]+\\.)?readly\\.(com|co)/", 22, [1641, 1642], [{ e: 1643 }], [{ check: "any", v: 1643 }], [{ if: { e: 1644 }, then: [{ c: 1644 }], else: [{ c: 1645 }, { if: { e: 1646 }, then: [{ all: true, c: 1646 }], else: [] }, { c: 1647 }] }], [], {}], [1, "reddit.com", 2, "^https://(www|old)\\.reddit\\.com/", 22, [1648], [{ e: 1649 }], [{ v: 1649 }], [{ c: 1650 }], [{ cc: 1651 }], {}], [1, "remarkable.com", 1, "^https://(www\\.)?remarkable\\.com/", 22, [1652], [{ e: 1653 }], [{ v: 1653 }], [{ h: 1652 }], [], {}], [1, "roblox", 0, "^https://(www\\.)?roblox\\.com/", 10, [], [{ e: 1654 }], [{ v: 1655 }], [{ c: 1656 }], [{ cc: 1657 }], {}], [1, "rog-forum.asus.com", 2, "^https://rog-forum\\.asus\\.com/", 22, [1658], [{ e: 1658 }], [{ v: 1658 }], [{ k: 1659 }, { c: 1660 }], [], {}], [1, "roofingmegastore.co.uk", 2, "^https://(www\\.)?roofingmegastore\\.co\\.uk", 22, [1661], [{ e: 1661 }], [{ v: 1661 }], [{ k: 1662 }, { c: 1663 }], [], {}], [1, "rspb.org.uk", 0, "^https://(\\w+\\.)?rspb\\.org\\.uk/", 10, [1664], [{ e: 1664 }], [{ v: 1664 }], [{ c: 1665 }, { wv: 1666 }, { c: 1667 }], [{ cc: 1668 }], {}], [1, "rt", 1, "^https://(www\\.)?rt\\.com/", 22, [1669], [{ e: 1669 }], [{ v: 1669 }], [{ h: 1669 }], [], {}], [1, "rug-nl", 0, "^https?://(\\w+\\.)?rug\\.nl/", 10, [1670], [{ e: 1671 }], [{ v: 1670 }], [{ c: 1671 }], [], {}], [1, "rule34-xxx", 2, "^https?://(www\\.)?rule34\\.xxx/", 22, [1672], [{ e: 1672 }], [{ v: 1672 }], [{ c: 1673 }], [{ cc: 1674 }], {}], [1, "ryanair", 0, "^https://(www\\.)?ryanair\\.com/", 10, [1675], [{ e: 1675 }], [{ v: 1675 }], [{ c: 1676 }], [{ cc: 1677 }], {}], [1, "samsung.com", 1, "^https://www\\.samsung\\.com/", 22, [1678], [{ e: 1678 }], [{ v: 1678 }], [{ h: 1678 }], [], {}], [1, "schoolhouse-com", 1, "^https://(www\\.)?schoolhouse\\.com/", 22, [1679], [{ e: 1679 }], [{ v: 1679 }], [{ h: 1679 }], [], {}], [1, "scmp", 2, "^https://(www\\.)?scmp\\.com/", 22, [1680], [{ any: [{ e: 1681 }, { e: 413 }] }], [{ any: [{ v: 1681 }, { v: 1682 }] }], [{ timeout: 4e3, optional: true, wv: 1682 }, { if: { v: 1682 }, then: [{ k: 415 }, { all: true, optional: true, k: 416 }, { optional: true, k: 417 }] }, { timeout: 3e3, optional: true, wv: 1681 }, { h: 1680 }], [], {}], [1, "sex.com", 2, "^https?://(\\w+\\.)?sex\\.com/", 22, [1683], [{ e: 1684 }], [{ v: 1685 }], [{ w: 1686 }, { c: 1687 }, { c: 1688 }], [{ cc: 1689 }], {}], [1, "shein.com", 2, "^https?://([a-z]+\\.)?shein\\.com/", 22, [1690], [{ e: 1691 }], [{ v: 1691 }], [{ c: 1691 }], [{ timeout: 1e3, check: "none", wv: 1691 }], {}], [1, "simplehuman-com", 2, "^https?://(www\\.)?simplehuman\\.com/", 10, [1692], [{ e: 1693 }], [{ v: 1694 }], [{ c: 1693 }, { c: 1695 }], [], {}], [1, "simyo-nl", 2, "^https?://(www\\.)?simyo\\.nl/", 10, [1696], [{ v: 1696 }], [{ v: 1696 }], [{ c: 1697 }], [{ cc: 1698 }], {}], [1, "skyscanner", 0, "^https://(www\\.)?skyscanner[\\.a-z]+/", 10, [1654], [{ e: 1699 }], [{ v: 1699 }], [{ c: 1700 }, { check: "none", wv: 1699 }], [{ eval: "EVAL_SKYSCANNER_TEST" }], {}], [1, "sostereo.com", 2, "^https://(www\\.)?sostereo\\.com/", 22, [1701], [{ e: 1701 }], [{ v: 1701 }], [{ c: 1702 }, { w: 1703 }, { all: true, optional: true, k: 1704 }, { c: 1705 }], [], {}], [1, "strato.de", 2, "^https://www\\.strato\\.de/", 22, [1706], [{ e: 1707 }], [{ v: 1707 }], [{ k: 1708 }, { c: 1709 }], [], {}], [1, "svt.se", 2, "^https://www\\.svt\\.se/", 22, [1710], [{ e: 1710 }], [{ v: 1711 }], [{ c: 1712 }], [{ cc: 1713 }], {}], [1, "temu", 2, "^https://([a-z0-9-]+\\.)?temu\\.com/", 22, [], [{ e: 1714 }], [{ v: 1714 }], [{ if: { e: 1715 }, then: [{ c: 1715 }], else: [{ c: 1716 }] }], [], {}], [1, "tesco", 0, "^https://(www\\.)?tesco\\.com/", 22, [1717], [{ e: 1717 }], [{ v: 1717 }], [{ w: 1492 }, { c: 1718 }], [{ cc: 1719 }], {}], [1, "tesla", 2, "^https://(www\\.)?tesla\\.com/", 10, [], [{ e: 1720 }], [{ v: 1720 }], [{ c: 1721 }], [{ cc: 1722 }], {}], [3, "theguardian.com", 1, "^https?://(\\w+\\.)?theguardian\\.com/", 10, [1723], [{ e: 1723 }], [{ any: [{ v: 1723 }, { e: 1724 }] }], [{ h: 1723 }, { stylesheet: "html.sp-message-open body { position: static !important; top: auto !important; width: auto !important; overflow: auto !important; }", stylesheetId: "theguardian-scroll-unlock" }], [{ check: "none", v: 1723 }], {}], [1, "theinfatuation", 2, "^https://(www\\.)?theinfatuation\\.com", 10, [1725], [{ e: 1726 }], [{ v: 1726 }], [{ c: 1726 }], [{ check: "none", v: 1725 }], {}], [1, "tinyurl", 1, "^https://tinyurl\\.com/", 22, [1727], [{ e: 1727 }], [{ v: 1727 }], [{ h: 1727 }], [], {}], [1, "tlc-direct", 2, "^https?://(\\w+\\.)?tlc-direct\\.co\\.uk/", 22, [1408], [{ e: 1728 }, { e: 1729 }], [{ v: 1408 }], [{ c: 1730 }, { if: { e: 1731 }, then: [{ k: 1732 }] }, { if: { e: 1733 }, then: [{ k: 1734 }] }, { c: 1735 }], [{ cc: 1736 }], {}], [1, "toho-one.com", 2, "^https?://www\\.toho-one\\.com/", 22, [], [{ e: 1737 }], [{ v: 1737 }], [{ c: 1738 }, { waitFor: ["xpath///div[contains(@class, 'fixed') and .//button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']]", "input[type=checkbox]:checked:not(:disabled)"] }, { click: ["xpath///div[contains(@class, 'fixed') and .//button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']]", "input[type=checkbox]:checked:not(:disabled)"], all: true }, { c: 1739 }], [{ timeout: 1e3, check: "none", wv: 1737 }], {}], [1, "tohotheater-jp", 2, "^https?://(?:[^/]+\\.)?tohotheater\\.jp/", 22, [1740], [{ e: 1741 }], [{ v: 1740 }], [{ c: 1741 }, { c: 1742 }, { c: 1743 }, { c: 1744 }, { c: 1745 }, { c: 1746 }], [{ cc: 1747 }, { cc: 1748 }], {}], [1, "track.amazon.com", 2, "^https://[a-z]*\\.amazon\\.", 22, [1749], [{ e: 1750 }], [{ v: 1750 }], [{ k: 1751 }], [], {}], [1, "transip-nl", 2, "^https://www\\.transip\\.nl/", 22, [1752], [{ any: [{ e: 1752 }, { e: 1753 }] }], [{ any: [{ v: 1752 }, { v: 1753 }] }], [{ if: { e: 1753 }, then: [{ k: 1754 }], else: [{ k: 1755 }] }], [], {}], [1, "truecar", 1, "^https://(www\\.)?truecar\\.com/", 22, [1756], [{ e: 1757 }], [{ v: 1757 }], [{ h: 1756 }], [], {}], [1, "twitch.tv", 2, "^https?://([\\w-]+\\.)?twitch\\.tv/", 22, [1758], [{ e: 1759 }], [{ v: 1759 }], [{ if: { e: 1760 }, then: [{ retry: 3, retryInterval: 1e3, c: 1761 }, { check: "none", wv: 1759 }], else: [{ c: 1762 }, { w: 1763 }, { all: true, optional: true, k: 1764 }, { c: 1765 }, { check: "none", wv: 1766 }] }], [], {}], [1, "twitter", 2, "^https://([a-z0-9-]+\\.)?(twitter|x)\\.com/", 22, [1767], [{ e: 1768 }], [{ v: 1768 }], [{ c: 1768 }], [{ timeout: 1e3, check: "none", wv: 1768 }], {}], [1, "unicourt", 1, "^https://(www\\.)?unicourt\\.com/", 22, [1769], [{ e: 1769 }], [{ v: 1769 }], [{ h: 1769 }], [], {}], [1, "unive-nl", 0, "^https?://(?:[^/]+\\.)?unive\\.nl/", 10, [1770], [{ e: 1771 }], [{ v: 1770 }], [{ c: 1772 }], [{ check: "none", timeout: 2e3, wv: 1770 }], {}], [1, "uswitch.com", 2, "^https://(www\\.)?uswitch\\.com/", 10, [1773], [{ e: 1774 }], [{ v: 1774 }], [{ c: 1775 }], [], {}], [1, "uwv-nl", 0, "^https?://(?:www\\.)?uwv\\.nl/", 10, [1776], [{ e: 1776 }], [{ v: 1777 }], [{ c: 1777 }], [{ check: "none", timeout: 2e3, wv: 1777 }], {}], [1, "venngage.com", 2, "^https?://(\\w+\\.)?venngage\\.com/", 22, [1778, 1779], [{ e: 1780 }, { v: 1781 }], [{ v: 1781 }], [{ c: 1780 }, { c: 1782 }], [{ cc: 1783 }], {}], [1, "vmock", 2, "^https?://(\\w+\\.)?vmock\\.com/", 22, [1784, 1785], [{ e: 1786 }], [{ v: 1786 }], [{ c: 1786 }, { c: 1787 }, { timeout: 2e3, check: "none", wv: 1785 }], [{ cc: 1788 }], {}], [1, "vodafone.de", 2, "^https://www\\.vodafone\\.de/", 22, [1789], [{ e: 1790 }], [{ v: 1791 }], [{ k: 1792 }], [], {}], [1, "volvocars-com", 2, "^https://([a-z0-9-]+\\.)?volvocars\\.com/", 10, [1793], [{ exists: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"] }], [{ visible: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"] }], [{ wait: 500 }, { waitForThenClick: ["cookie-banner#cookie-banner-host", "#onetrust-reject-all-handler"] }, { waitForVisible: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"], check: "none", timeout: 5e3 }], [{ cc: 1794 }], {}], [1, "walmart-ca", 0, "^https?://(www\\.)?walmart\\.ca/", 22, [], [{ e: 1795 }], [{ v: 1795 }], [{ retry: 5, retryInterval: 500, c: 1796 }, { c: 1797 }], [{ check: "none", timeout: 5e3, wv: 1795 }], {}], [1, "wikiwand", 1, "^https://(www\\.)?wikiwand\\.com/", 22, [1798], [{ e: 1798 }], [{ v: 1798 }], [{ h: 1798 }], [], {}], [1, "xe.com", 2, "^https?://(www\\.)?xe\\.com/", 22, [1799, 1800], [{ e: 1801 }], [{ v: 1801 }], [{ retry: 3, retryInterval: 500, c: 1802 }, { wv: 1803 }, { c: 1804 }], [{ cc: 1805 }], {}], [1, "xhamster-eu", 2, "^https://(\\w+\\.)?xhamster\\d?\\.com", 22, [1806], [{ e: 1807 }], [{ v: 1807 }], [{ wv: 1808 }, { wait: 500 }, { k: 1808 }, { optional: true, h: 1809 }], [], {}], [1, "xhamster-us", 1, "^https://(\\w+\\.)?xhamster\\d?\\.com", 22, [1810], [{ e: 1810 }], [{ v: 1811 }], [{ h: 1810 }], [], {}], [1, "xvideos", 2, "^https://[a-z]*\\.xvideos\\.com/", 22, [], [{ e: 907 }], [{ v: 908 }], [{ c: 909 }], [], {}], [1, "yachtclubgames.com", 2, "^https?://(www\\.)?yachtclubgames\\.com/", 22, [1812], [{ e: 1812 }], [{ v: 1812 }], [{ c: 1813 }], [{ timeout: 1e3, check: "none", wv: 1812 }], {}], [1, "Yahoo", 2, "^https://consent\\.yahoo\\.com/v2/", 22, [798], [{ e: 1814 }], [{ v: 1814 }], [{ c: 1815 }], [], {}], [1, "zentralruf-de", 2, "^https://(www\\.)?zentralruf\\.de", 22, [1816], [{ e: 1816 }], [{ v: 1816 }], [{ c: 1817 }], [], {}], [1, "zinio", 0, "^https://(www\\.)?zinio\\.com/", 22, [], [{ e: 1818 }], [{ v: 1818 }], [{ c: 1819 }, { c: 1820 }], [{ cc: 1821 }], {}]], index: { genericRuleRange: [0, 227], frameRuleRange: [225, 254], specificRuleRange: [227, 843], genericStringEnd: 928, frameStringEnd: 970 } };

  // standalone/content.ts
  function isMainFrame() {
    return window.top === window;
  }
  function buildRules() {
    return {
      autoconsent: [],
      compact: filterCompactRules(compact_rules_default, {
        url: window.location.href,
        mainFrame: isMainFrame()
      })
    };
  }
  function logMessage(message) {
    switch (message.type) {
      case "cmpDetected":
        console.log(`autoconsent detected CMP: ${message.cmp}`);
        break;
      case "popupFound":
        console.log(`autoconsent found popup: ${message.cmp}`);
        break;
      case "optOutResult":
        console.log(`autoconsent opt-out result: ${message.cmp}`, message.result);
        break;
      case "autoconsentDone":
        console.log(`autoconsent done: ${message.cmp}`, {
          duration: message.duration,
          totalClicks: message.totalClicks
        });
        break;
      case "autoconsentError":
        console.warn("autoconsent error", message.details);
        break;
    }
  }
  if (!window.autoconsentReceiveMessage) {
    const config = {
      isMainWorld: true,
      enableHeuristicDetection: true,
      heuristicMode: "tier2",
      enablePopupMutationObserver: false,
      logs: {
        lifecycle: true,
        rulesteps: true,
        detectionsteps: false,
        evals: false,
        errors: true,
        messages: false,
        waits: true
      }
    };
    const rules = buildRules();
    const messages = [];
    const consentRef = {};
    window.autoconsentReceiveMessage = async (message) => {
      if (!consentRef.current) {
        throw new Error("autoconsent is not initialized yet");
      }
      await consentRef.current.receiveMessageCallback(message);
    };
    const sendMessage = async (message) => {
      messages.push(message);
      logMessage(message);
    };
    const consent = new AutoConsent(sendMessage);
    consentRef.current = consent;
    window.autoconsentStandalone = {
      instance: consent,
      messages
    };
    consent.initialize(config, rules);
    console.log("autoconsent standalone initialized", {
      instanceId: consent.id,
      rules: rules.compact?.r.length ?? 0,
      url: window.location.href
    });
  } else {
    console.warn("autoconsent already initialized", window.autoconsentReceiveMessage);
  }
})();
