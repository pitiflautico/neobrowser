// ═══════════════════════════════════════════════════════════════
// FORMS — CSRF-aware form analysis and filling.
// Loaded AFTER extract.js. Complements __neo_extract_form_schema.
// ═══════════════════════════════════════════════════════════════

// Helper: find the label text for an input element
function __neo_find_label(input) {
    // label[for=id]
    if (input.id) {
        try {
            const label = document.querySelector('label[for="' + input.id + '"]');
            if (label) return label.textContent.trim();
        } catch {}
    }
    // Parent label
    const parent = input.closest ? input.closest('label') : null;
    if (parent) return parent.textContent.trim();
    // aria-label
    return (input.getAttribute ? input.getAttribute('aria-label') : '') || '';
}

// Analyze all forms on the page, detecting CSRF tokens and hidden fields
globalThis.__neo_analyze_forms = function() {
    var forms = document.querySelectorAll('form');
    var result = [];
    for (var fi = 0; fi < forms.length; fi++) {
        var form = forms[fi];
        var inputs = Array.from(form.querySelectorAll('input, select, textarea'));

        // Detect CSRF token field
        var csrf = null;
        for (var i = 0; i < inputs.length; i++) {
            var inp = inputs[i];
            var n = (inp.name || '') + '|' + (inp.id || '');
            if (/csrf|_token|authenticity.token|nonce|xsrf|__RequestVerificationToken/i.test(n)) {
                csrf = inp;
                break;
            }
        }

        // Also check meta tags for CSRF (Rails, Laravel, Django patterns)
        var csrfMeta = null;
        if (!csrf) {
            var metaCsrf = document.querySelector('meta[name="csrf-token"]')
                || document.querySelector('meta[name="csrf-param"]')
                || document.querySelector('meta[name="_token"]');
            if (metaCsrf) {
                csrfMeta = {
                    name: metaCsrf.getAttribute('name'),
                    value: metaCsrf.getAttribute('content') || ''
                };
            }
        }

        var hiddenFields = [];
        var visibleFields = [];
        for (var j = 0; j < inputs.length; j++) {
            var el = inputs[j];
            if (el.type === 'hidden') {
                hiddenFields.push({name: el.name || '', value: el.value || ''});
            } else {
                var field = {
                    name: el.name || el.id || '',
                    type: el.type || 'text',
                    tag: el.tagName.toLowerCase(),
                    required: el.required || false,
                    placeholder: el.placeholder || '',
                    label: __neo_find_label(el),
                    value: el.value || ''
                };
                if (el.tagName === 'SELECT') {
                    field.options = Array.from(el.querySelectorAll('option')).map(function(o) {
                        return {value: o.value, text: (o.textContent || '').trim()};
                    });
                }
                visibleFields.push(field);
            }
        }

        result.push({
            action: form.action || form.getAttribute('action') || '',
            method: (form.method || 'GET').toUpperCase(),
            id: form.id || null,
            name: form.getAttribute('name') || null,
            has_csrf: !!(csrf || csrfMeta),
            csrf_field: csrf ? {name: csrf.name, value: csrf.value} : csrfMeta,
            hidden_fields: hiddenFields,
            fields: visibleFields
        });
    }
    return JSON.stringify(result);
};

// Fill a form by selector, preserving CSRF tokens and hidden fields.
// formSelector: CSS selector for the form
// fields: JSON string of {fieldName: value, ...}
globalThis.__neo_fill_form = function(formSelector, fieldsJson) {
    var form = document.querySelector(formSelector);
    if (!form) return JSON.stringify({ok: false, error: 'form not found: ' + formSelector});

    var fields;
    try {
        fields = typeof fieldsJson === 'string' ? JSON.parse(fieldsJson) : fieldsJson;
    } catch (e) {
        return JSON.stringify({ok: false, error: 'invalid fields JSON: ' + e.message});
    }

    var results = [];
    var keys = Object.keys(fields);
    for (var k = 0; k < keys.length; k++) {
        var key = keys[k];
        var value = fields[key];
        var input = null;

        // 1. By name
        try { input = form.querySelector('[name="' + key + '"]'); } catch {}
        // 2. By id
        if (!input) {
            try { input = form.querySelector('#' + CSS.escape(key)); } catch {}
            if (!input) try { input = form.querySelector('#' + key); } catch {}
        }
        // 3. By placeholder (case-insensitive via iteration)
        if (!input) {
            var allInputs = form.querySelectorAll('input, select, textarea');
            for (var i = 0; i < allInputs.length; i++) {
                var ph = allInputs[i].placeholder || '';
                if (ph.toLowerCase().indexOf(key.toLowerCase()) !== -1) {
                    input = allInputs[i];
                    break;
                }
            }
        }
        // 4. By label text
        if (!input) {
            var labels = form.querySelectorAll('label');
            for (var li = 0; li < labels.length; li++) {
                var labelText = (labels[li].textContent || '').toLowerCase();
                if (labelText.indexOf(key.toLowerCase()) !== -1) {
                    var forId = labels[li].getAttribute('for');
                    if (forId) {
                        try { input = form.querySelector('#' + forId); } catch {}
                    }
                    if (!input) input = labels[li].querySelector('input, select, textarea');
                    if (input) break;
                }
            }
        }

        if (input) {
            // Handle select elements
            if (input.tagName === 'SELECT') {
                input.value = value;
            }
            // Handle checkboxes/radios
            else if (input.type === 'checkbox' || input.type === 'radio') {
                input.checked = !!value;
            }
            // Handle text-like inputs
            else {
                input.value = value;
            }
            // Fire events for SPA frameworks (React, Vue, Angular)
            try {
                input.dispatchEvent(new Event('input', {bubbles: true}));
                input.dispatchEvent(new Event('change', {bubbles: true}));
                // React 16+ uses native input setter
                var nativeInputValueSetter = Object.getOwnPropertyDescriptor(
                    window.HTMLInputElement ? window.HTMLInputElement.prototype : {},
                    'value'
                );
                if (nativeInputValueSetter && nativeInputValueSetter.set && input.tagName === 'INPUT') {
                    nativeInputValueSetter.set.call(input, value);
                    input.dispatchEvent(new Event('input', {bubbles: true}));
                }
            } catch {}
            results.push({field: key, ok: true});
        } else {
            results.push({field: key, ok: false, error: 'not found'});
        }
    }

    var failed = results.filter(function(r) { return !r.ok; });
    return JSON.stringify({
        ok: failed.length === 0,
        filled: results.filter(function(r) { return r.ok; }).length,
        total: results.length,
        results: results
    });
};

// Submit a form, collecting all fields (including hidden/CSRF) into a structured object.
// Returns the form data that would be submitted (for Rust-side HTTP submission).
globalThis.__neo_submit_form = function(formSelector) {
    var form = document.querySelector(formSelector);
    if (!form) return JSON.stringify({ok: false, error: 'form not found: ' + formSelector});

    var data = {};
    var inputs = form.querySelectorAll('input, select, textarea');
    for (var i = 0; i < inputs.length; i++) {
        var el = inputs[i];
        var name = el.name;
        if (!name) continue;

        if (el.type === 'checkbox') {
            if (el.checked) data[name] = el.value || 'on';
        } else if (el.type === 'radio') {
            if (el.checked) data[name] = el.value;
        } else if (el.tagName === 'SELECT' && el.multiple) {
            var selected = Array.from(el.selectedOptions).map(function(o) { return o.value; });
            data[name] = selected;
        } else {
            data[name] = el.value || '';
        }
    }

    return JSON.stringify({
        ok: true,
        action: form.action || form.getAttribute('action') || '',
        method: (form.method || 'GET').toUpperCase(),
        enctype: form.enctype || 'application/x-www-form-urlencoded',
        data: data
    });
};
