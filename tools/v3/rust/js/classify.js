// classify.js — Automatic page type classification.
// Returns one of: search_results, login, article, data_table, form, homepage, product, checkout, content

globalThis.__neo_classify = function() {
    var url = (typeof location !== 'undefined' && location.href) || '';
    var title = (typeof document !== 'undefined' && document.title) || '';
    var forms = document.querySelectorAll('form').length;
    var inputs = document.querySelectorAll('input').length;
    var articles = document.querySelectorAll('article').length;
    var tables = document.querySelectorAll('table').length;
    var results = document.querySelectorAll('[class*="result"],[class*="search"],[data-result]').length;
    var passwordInputs = document.querySelectorAll('input[type="password"]').length;
    var products = document.querySelectorAll('[class*="product"],[class*="price"],[data-product]').length;
    var carts = document.querySelectorAll('[class*="cart"],[class*="checkout"],[class*="basket"]').length;

    // Search results
    if (url.indexOf('/search') !== -1 || url.indexOf('?q=') !== -1 || results > 3) return 'search_results';

    // Login / auth
    if (url.indexOf('/login') !== -1 || url.indexOf('/signin') !== -1 ||
        url.indexOf('/sign-in') !== -1 || url.indexOf('/auth') !== -1 ||
        passwordInputs > 0 || (title.toLowerCase().indexOf('login') !== -1) ||
        (title.toLowerCase().indexOf('sign in') !== -1)) return 'login';

    // Checkout / cart
    if (url.indexOf('/checkout') !== -1 || url.indexOf('/cart') !== -1 || carts > 2) return 'checkout';

    // Product page
    if (url.indexOf('/product') !== -1 || products > 3) return 'product';

    // Article / blog
    if (articles > 0 || document.querySelector('article')) return 'article';

    // Data tables
    if (tables > 2) return 'data_table';

    // Form-heavy page
    if (forms > 0 && inputs > 3) return 'form';

    // Homepage detection
    try {
        var path = new URL(url).pathname;
        if (path === '/' || path === '') return 'homepage';
    } catch (e) {
        if (url.match(/^https?:\/\/[^\/]+\/?$/)) return 'homepage';
    }

    return 'content';
};
