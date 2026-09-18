// JioTV+ login dialog and the "only JioTV+ channels" filter.
// Needs utils.js (postJSON, getLocalStorageItem, setLocalStorageItem).

const TVPLUS_ONLY_STORAGE_KEY = "tvplusOnly";

function tvPlusShowStep(step) {
  ["number", "connection", "otp"].forEach((name) => {
    const el = document.getElementById(`tvplus-step-${name}`);
    if (el) el.classList.toggle("hidden", name !== step);
  });
  const focusTarget = { number: "tvplus-number", otp: "tvplus-otp" }[step];
  if (focusTarget) document.getElementById(focusTarget)?.focus();
}

function tvPlusMessage(text) {
  const el = document.getElementById("tvplus-message");
  if (el) el.textContent = text;
}

function tvPlusNumber() {
  return (document.getElementById("tvplus-number")?.value || "").trim();
}

function tvPlusOpen() {
  tvPlusMessage("");
  tvPlusShowStep("number");
  document.getElementById("tvplus_modal")?.showModal();
}

function tvPlusRenderConnections(connections) {
  const list = document.getElementById("tvplus-connections");
  if (!list) return;
  list.replaceChildren();
  connections.forEach((conn, i) => {
    const label = document.createElement("label");
    label.className = "flex cursor-pointer items-center gap-2.5 rounded-lg border border-base-300 p-2";

    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "tvplus-connection";
    radio.value = String(conn.index);
    radio.className = "radio radio-sm radio-primary";
    radio.checked = i === 0;

    const text = document.createElement("span");
    text.className = "text-sm";
    const parts = [conn.name, conn.product, conn.lineEndsWith ? `line ending ${conn.lineEndsWith}` : ""];
    text.textContent = parts.filter(Boolean).join(" · ");

    label.append(radio, text);
    list.append(label);
  });
}

async function tvPlusSendOTP() {
  const number = tvPlusNumber();
  if (!/^[0-9]{10}$/.test(number)) {
    tvPlusMessage("Enter the 10-digit mobile number.");
    return;
  }
  tvPlusMessage("");
  try {
    const data = await postJSON("/tvplus/login/sendOTP", { number });
    if (data.connections && data.connections.length) {
      tvPlusRenderConnections(data.connections);
      tvPlusShowStep("connection");
    } else if (data.status) {
      tvPlusShowStep("otp");
    } else {
      tvPlusMessage(data.message || "We couldn’t send the OTP. Check the number and try again.");
    }
  } catch (err) {
    console.log(err);
    tvPlusMessage("We couldn’t reach the server. Try again.");
  }
}

async function tvPlusChooseConnection() {
  const picked = document.querySelector('input[name="tvplus-connection"]:checked');
  if (!picked) {
    tvPlusMessage("Choose a connection.");
    return;
  }
  tvPlusMessage("");
  try {
    const data = await postJSON("/tvplus/login/sendOTP", {
      number: tvPlusNumber(),
      connection: Number(picked.value),
    });
    if (data.status) {
      tvPlusShowStep("otp");
    } else {
      tvPlusMessage(data.message || "We couldn’t send the OTP. Try again.");
    }
  } catch (err) {
    console.log(err);
    tvPlusMessage("We couldn’t reach the server. Try again.");
  }
}

async function tvPlusVerifyOTP() {
  const otp = (document.getElementById("tvplus-otp")?.value || "").trim();
  if (!otp) {
    tvPlusMessage("Enter the OTP.");
    return;
  }
  tvPlusMessage("");
  try {
    const data = await postJSON("/tvplus/login/verifyOTP", { number: tvPlusNumber(), otp });
    if (data.status) {
      document.getElementById("tvplus_modal")?.close();
      window.location.reload();
    } else {
      tvPlusMessage(data.message || "The OTP is incorrect or expired. Try again.");
    }
  } catch (err) {
    console.log(err);
    tvPlusMessage("We couldn’t reach the server. Try again.");
  }
}

// Filtering uses a class on <body> because the search filter owns each
// card's inline style.display.
function toggleTVPlusOnly(only) {
  document.body.classList.toggle("tvplus-only", only);
  setLocalStorageItem(TVPLUS_ONLY_STORAGE_KEY, only);
}

function initTVPlusFilter() {
  const count = document.querySelectorAll('.card[data-tvplus="true"]').length;
  const toggle = document.getElementById("tvplus-only-toggle");
  if (!toggle) return;
  const label = toggle.closest("label");
  if (!count) {
    if (label) label.style.display = "none";
    document.body.classList.remove("tvplus-only");
    return;
  }
  if (label) label.style.display = "";
  const countLabel = document.getElementById("tvplus-channel-count");
  if (countLabel) countLabel.textContent = `${count} JioTV+ channels`;
  const stored = getLocalStorageItem(TVPLUS_ONLY_STORAGE_KEY, false) === true;
  toggle.checked = stored;
  toggleTVPlusOnly(stored);
}

initTVPlusFilter();
