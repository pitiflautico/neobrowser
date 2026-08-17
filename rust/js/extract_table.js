// Turn an HTML table into rows and columns.
//
// Headers come from `<th>` where present and from the first row otherwise, because plenty of
// real tables never use `<th>` — and a table whose first row is silently treated as data
// produces a column set of `["0","1","2"]`, which is useless.

(function() {
    var tables = document.querySelectorAll(__SEL__);
    var table = tables[__IDX__];
    if (!table) return JSON.stringify([]);
    var headers = Array.from(table.querySelectorAll('th')).map(function(th){ return th.textContent.trim(); });
    if (!headers.length) {
        var firstRow = table.querySelector('tr');
        if (firstRow) headers = Array.from(firstRow.querySelectorAll('td')).map(function(td){ return td.textContent.trim(); });
    }
    var rows = Array.from(table.querySelectorAll('tr')).slice(headers.length ? 1 : 0);
    var data = rows.map(function(row) {
        var cells = Array.from(row.querySelectorAll('td')).map(function(td){ return td.textContent.trim(); });
        var obj = {};
        cells.forEach(function(c, i){ obj[headers[i] || i] = c; });
        return obj;
    });
    return JSON.stringify(data);
})()
