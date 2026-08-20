// Which frames JS can actually enter.
//
// Checked from the page rather than inferred from URLs: the same-origin comparison Chrome
// applies is the only authority on it, and a URL that looks same-origin may not be.
(function () {
  var out = [];
  var frames = document.querySelectorAll('iframe,frame');
  for (var i = 0; i < frames.length; i++) {
    var accessible = false;
    try {
      accessible = !!frames[i].contentDocument;
    } catch (e) {
      accessible = false;
    }
    out.push({
      src: (frames[i].getAttribute('src') || '(inline)').slice(0, 200),
      same_origin_accessible: accessible,
    });
  }
  return JSON.stringify(out);
})()
