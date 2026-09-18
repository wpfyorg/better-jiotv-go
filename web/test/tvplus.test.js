const { readFileSync } = require("node:fs");

const loadScripts = () => {
  window.eval(readFileSync("static/internal/utils.js", "utf8"));
  window.eval(readFileSync("static/internal/tvplus.js", "utf8"));
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const respond = (body) =>
  global.fetch.mockResolvedValueOnce({ json: () => Promise.resolve(body) });

const lastRequest = () => {
  const [url, options] = global.fetch.mock.calls[global.fetch.mock.calls.length - 1];
  return { url, body: JSON.parse(options.body) };
};

const visible = (id) => !document.getElementById(id).classList.contains("hidden");

describe("JioTV+ login dialog", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.className = "";
    document.body.innerHTML = `
      <dialog id="tvplus_modal">
        <div id="tvplus-step-number"><input id="tvplus-number" /></div>
        <div id="tvplus-step-connection" class="hidden"><div id="tvplus-connections"></div></div>
        <div id="tvplus-step-otp" class="hidden"><input id="tvplus-otp" /></div>
        <p id="tvplus-message"></p>
      </dialog>
    `;
    const dialog = document.getElementById("tvplus_modal");
    dialog.showModal = jest.fn();
    dialog.close = jest.fn();
    global.fetch = jest.fn();
    global.console.log = jest.fn();
    loadScripts();
  });

  test("rejects a number that is not 10 digits without calling the server", async () => {
    document.getElementById("tvplus-number").value = "12345";
    await tvPlusSendOTP();
    expect(global.fetch).not.toHaveBeenCalled();
    expect(document.getElementById("tvplus-message").textContent).toMatch(/10-digit/);
  });

  test("lists fibre connections as text and sends the chosen one", async () => {
    document.getElementById("tvplus-number").value = "9000000000";
    respond({
      status: true,
      connections: [
        { index: 0, name: "<img src=x onerror=alert(1)>", product: "JIO HOME VOICE", lineEndsWith: "0001" },
        { index: 1, name: "Test", product: "JIO HOME VOICE", lineEndsWith: "0002" },
      ],
    });
    await tvPlusSendOTP();

    expect(lastRequest()).toEqual({ url: "/tvplus/login/sendOTP", body: { number: "9000000000" } });
    expect(visible("tvplus-step-connection")).toBe(true);
    const list = document.getElementById("tvplus-connections");
    expect(list.querySelector("img")).toBeNull();
    expect(list.textContent).toContain("line ending 0002");

    list.querySelectorAll('input[name="tvplus-connection"]')[1].checked = true;
    respond({ status: true });
    await tvPlusChooseConnection();

    expect(lastRequest().body).toEqual({ number: "9000000000", connection: 1 });
    expect(visible("tvplus-step-otp")).toBe(true);
    expect(visible("tvplus-step-connection")).toBe(false);
  });

  test("goes straight to the OTP step when there is no connection to choose", async () => {
    document.getElementById("tvplus-number").value = "9000000000";
    respond({ status: true });
    await tvPlusSendOTP();
    expect(visible("tvplus-step-otp")).toBe(true);
  });

  test("shows the server's message when the OTP is rejected", async () => {
    document.getElementById("tvplus-number").value = "9000000000";
    document.getElementById("tvplus-otp").value = "123456";
    respond({ message: "Send the OTP first" });
    await tvPlusVerifyOTP();

    expect(lastRequest()).toEqual({
      url: "/tvplus/login/verifyOTP",
      body: { number: "9000000000", otp: "123456" },
    });
    expect(document.getElementById("tvplus-message").textContent).toBe("Send the OTP first");
    expect(document.getElementById("tvplus_modal").close).not.toHaveBeenCalled();
  });

  test("reports a network failure", async () => {
    document.getElementById("tvplus-number").value = "9000000000";
    const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
    global.fetch.mockRejectedValueOnce(new Error("offline"));
    await tvPlusSendOTP();
    errorSpy.mockRestore();
    await flush();
    expect(document.getElementById("tvplus-message").textContent).toMatch(/reach the server/);
  });
});

describe("JioTV+ channel filter", () => {
  const setup = (cards, { keepStorage = false } = {}) => {
    if (!keepStorage) localStorage.clear();
    document.body.className = "";
    document.body.innerHTML = `
      <label style="display: none;"><input id="tvplus-only-toggle" type="checkbox" /><span id="tvplus-channel-count"></span></label>
      ${cards}
    `;
  };

  test("stays hidden when there are no JioTV+ channels", () => {
    setup('<a class="card" data-channel-id="143"></a>');
    loadScripts();
    expect(document.getElementById("tvplus-only-toggle").closest("label").style.display).toBe("none");
    expect(document.body.classList.contains("tvplus-only")).toBe(false);
  });

  test("shows the count and remembers the choice", () => {
    setup(`
      <a class="card" data-channel-id="143"></a>
      <a class="card" data-channel-id="tvp_302084" data-tvplus="true"></a>
      <a class="card" data-channel-id="tvp_300396" data-tvplus="true"></a>
    `);
    loadScripts();
    const toggle = document.getElementById("tvplus-only-toggle");
    expect(toggle.closest("label").style.display).toBe("");
    expect(document.getElementById("tvplus-channel-count").textContent).toBe("2 JioTV+ channels");

    toggleTVPlusOnly(true);
    expect(document.body.classList.contains("tvplus-only")).toBe(true);

    setup('<a class="card" data-tvplus="true"></a>', { keepStorage: true });
    loadScripts();
    expect(document.getElementById("tvplus-only-toggle").checked).toBe(true);
    expect(document.body.classList.contains("tvplus-only")).toBe(true);
  });
});
