(() => {
  'use strict';

  // Initialization scripts run on every top-level navigation, including OAuth
  // providers. Never touch anything that is not Notion.
  const host = location.hostname;
  const isNotion =
    host === 'notion.so' ||
    host.endsWith('.notion.so') ||
    host === 'notion.com' ||
    host.endsWith('.notion.com') ||
    host === 'notion.site' ||
    host.endsWith('.notion.site');

  if (!isNotion || window.__renotion) return;
  window.__renotion = true;

  const css = __RENOTION_CSS__;

  const install = () => {
    const root = document.documentElement;
    if (!root) return false;

    root.setAttribute('data-renotion', '');
    if (document.getElementById('renotion-style')) return true;

    const style = document.createElement('style');
    style.id = 'renotion-style';
    style.textContent = css;
    (document.head || root).append(style);
    return true;
  };

  if (!install()) {
    // At document-start some engines have not created <html> yet.
    const observer = new MutationObserver(() => {
      if (install()) observer.disconnect();
    });
    observer.observe(document, { childList: true, subtree: true });
  }

  // Floating offline notification banner
  const updateNetworkStatus = () => {
    let pill = document.getElementById('renotion-offline-pill');
    if (!navigator.onLine) {
      if (!pill) {
        pill = document.createElement('div');
        pill.id = 'renotion-offline-pill';
        pill.innerHTML = '<span class="renotion-dot"></span><span>Offline — viewing cached content</span>';
        document.body ? document.body.appendChild(pill) : document.documentElement.appendChild(pill);
      }
      requestAnimationFrame(() => pill.classList.add('visible'));
    } else if (pill) {
      pill.classList.remove('visible');
    }
  };

  window.addEventListener('offline', updateNetworkStatus);
  window.addEventListener('online', updateNetworkStatus);
  if (!navigator.onLine) {
    document.addEventListener('DOMContentLoaded', updateNetworkStatus);
  }

  // Notion's "open in app" links use the notion:// scheme. The OS would hand
  // those to another client (or do nothing), so translate them into normal
  // navigation inside this window.
  document.addEventListener(
    'click',
    (event) => {
      const anchor =
        event.target instanceof Element
          ? event.target.closest('a[href^="notion://"]')
          : null;
      if (!anchor) return;

      const rest = (anchor.getAttribute('href') || '').slice('notion://'.length);
      if (!rest) return;

      event.preventDefault();
      event.stopPropagation();
      location.assign(/^https?:/i.test(rest) ? rest : `https://${rest}`);
    },
    true,
  );

  // Wire up the mouse's back/forward buttons, which WebKit leaves inert.
  window.addEventListener(
    'mousedown',
    (event) => {
      if (event.button === 3) {
        event.preventDefault();
        history.back();
      } else if (event.button === 4) {
        event.preventDefault();
        history.forward();
      }
    },
    true,
  );
})();
