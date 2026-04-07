import { React, ReactDOM, jsx } from "/vendor.js";

window.__oai_logHTML = () => console.log("HTML timing logged");
window.__oai_logTTI = () => console.log("TTI timing logged");

console.log("Entry module executing — calling hydrateRoot");
React.startTransition(() => {
  ReactDOM.hydrateRoot(document, jsx(React.StrictMode, {children: jsx("div", {id: "hydrated-app"})}));
});

console.log("Entry module complete");
