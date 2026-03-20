import { React, jsx } from "/vendor.js";
export default function Root() { return jsx("div", {id: "root"}); }
export function meta() { return [{title: "Root"}]; }
export function shouldRevalidate() { return false; }
