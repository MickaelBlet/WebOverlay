const banner = document.createElement("div");
banner.textContent = "Injected by WebOverlay";
Object.assign(banner.style, {
  position: "fixed",
  bottom: "12px",
  right: "12px",
  padding: "6px 10px",
  background: "#2563eb",
  color: "#fff",
  font: "13px system-ui, sans-serif",
  borderRadius: "6px",
  zIndex: 2147483647,
});
document.body.appendChild(banner);
console.log("[demo] banner added");
