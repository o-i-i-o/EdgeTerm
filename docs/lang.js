(function () {
  var root = document.documentElement;
  var btn = document.getElementById('langBtn');
  if (!btn) return;

  function apply(lang) {
    root.setAttribute('data-lang', lang);
    root.lang = lang === 'zh' ? 'zh-CN' : 'en';
    // The button names the language it switches to, so it is written in that
    // one: a screen reader should not read "中文" with an English voice.
    btn.textContent = lang === 'zh' ? 'EN' : '中文';
    btn.lang = lang === 'zh' ? 'en' : 'zh-CN';
    try { localStorage.setItem('edgeterm-lang', lang); } catch (e) {}
  }

  // The head script has already picked this visit's language before the first
  // paint; the button only has to show what it picked.
  apply(root.getAttribute('data-lang') === 'zh' ? 'zh' : 'en');

  btn.addEventListener('click', function () {
    apply(root.getAttribute('data-lang') === 'zh' ? 'en' : 'zh');
  });
})();
