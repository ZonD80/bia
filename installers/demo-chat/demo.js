// Store screenshots of bia's chat inside the apps (demo-chat/server.py serves it after link.js):
// bia's real web app with a made-up conversation, no bia behind it. The bus is replaced by a
// fake link that says hello and answers `hist`. ?demo=chat | call | attach picks the scene.
'use strict';
(() => {
  const scene = new URLSearchParams(location.search).get('demo') || 'chat';
  // the chat itself, also on a phone (an empty hash shows the chat list there)
  history.replaceState(null, '', location.pathname + location.search + '#chat/main');

  // this morning, before the status bars' 9:41 (iOS) and 10:00 (Android)
  const at = (hh, mm) => { const d = new Date(); d.setHours(hh, mm, 0, 0); return d.getTime(); };
  let n = 0;
  const msg = (from, ts, text, files) => ({ id: 'demo' + (++n), from, ts, text, files, read: true, rev: 1 });

  // files bia "sent": made in the page
  const FILES = {};
  const pdf = () => new Blob(['%PDF-1.4\n% demo\n'], { type: 'application/pdf' });
  FILES['f-tax'] = pdf();

  const MSGS = [
    msg('root', at(9, 12), 'Morning! What did I miss overnight?'),
    msg('bia', at(9, 13), 'Morning ☀️ Three things:\n\n'
      + '- Your sister wrote in the family group: dinner moved to **Saturday 19:30**.\n'
      + '- The flat on Carrer de Pujades dropped to **€1,150**. I asked the agent for a viewing; she offers Thursday 17:00.\n'
      + '- The tax summary you asked for is ready:'),
    msg('bia', at(9, 13), '', [{ fid: 'f-tax', name: 'tax-summary-2026.pdf', size: 248_312, mime: 'application/pdf', kind: 'document' }]),
    msg('root', at(9, 15), 'Great, book Thursday and remind me an hour before'),
    msg('bia', at(9, 16), 'Done: viewing on **Thursday at 17:00**, and I will write to you here at 16:00. I also sent the agent your questions about the deposit 👍'),
  ];

  BiaLink.create = function (opts) {
    const L = { state: 'connecting', rx: 1 };
    L.start = async () => {
      setTimeout(() => {
        L.state = 'online';
        opts.onState && opts.onState('online');
        opts.onHello && opts.onHello({ muted: false, typing: false });
        setTimeout(play, 600);
      }, 30);
    };
    L.act = async (t) => (t === 'hist' ? { messages: MSGS, more_before: false, rev: 1 } : {});
    L.send = async () => ({});
    L.url = f => (FILES[f.fid] ? Promise.resolve(URL.createObjectURL(FILES[f.fid])) : Promise.reject(new Error('no file')));
    L.blob = f => Promise.resolve(FILES[f]);
    L.retry = L.close = L.sendJson = () => {};
    return L;
  };

  // a photographed receipt, drawn here
  function receipt() {
    const c = document.createElement('canvas');
    c.width = 900; c.height = 1200;
    const g = c.getContext('2d');
    const bg = g.createLinearGradient(0, 0, 900, 1200);
    bg.addColorStop(0, '#8a7f72'); bg.addColorStop(1, '#5d554c');
    g.fillStyle = bg; g.fillRect(0, 0, 900, 1200);
    g.save();
    g.translate(450, 610); g.rotate(-0.04);
    g.shadowColor = 'rgba(0,0,0,.45)'; g.shadowBlur = 40; g.shadowOffsetY = 14;
    g.fillStyle = '#fbfaf6'; g.fillRect(-270, -500, 540, 1000);
    g.shadowColor = 'transparent';
    g.fillStyle = '#26221e'; g.textAlign = 'center';
    g.font = 'bold 42px Menlo, monospace'; g.fillText('CAFÈ DEL MAR', 0, -410);
    g.font = '24px Menlo, monospace';
    g.fillText('Pg. Marítim 12 · Barcelona', 0, -370);
    g.fillText('12/11/2026  13:42', 0, -335);
    g.textAlign = 'left';
    const rows = [['2 x Menú del dia', '31.00'], ['1 x Aigua 1L', '3.20'], ['2 x Cafè amb llet', '4.80'], ['1 x Crema catalana', '5.50']];
    let y = -250;
    g.font = '27px Menlo, monospace';
    for (const [a, b] of rows) { g.fillText(a, -230, y); g.textAlign = 'right'; g.fillText(b, 230, y); g.textAlign = 'left'; y += 52; }
    g.setLineDash([10, 8]); g.beginPath(); g.moveTo(-230, y - 10); g.lineTo(230, y - 10); g.strokeStyle = '#26221e'; g.lineWidth = 2; g.stroke();
    y += 40;
    g.font = 'bold 34px Menlo, monospace'; g.fillText('TOTAL', -230, y); g.textAlign = 'right'; g.fillText('€44.50', 230, y); g.textAlign = 'left';
    g.font = '24px Menlo, monospace'; y += 50;
    g.fillText('IVA 10% incl.', -230, y); g.textAlign = 'right'; g.fillText('4.05', 230, y); g.textAlign = 'left';
    y += 50; g.fillText('VISA **** 4821', -230, y);
    g.textAlign = 'center'; g.font = '26px Menlo, monospace'; g.fillText('Gràcies!', 0, 420);
    for (let i = 0; i < 46; i++) g.fillRect(-200 + i * 9, 350 - 0, i % 3 ? 3 : 6, 40);
    g.restore();
    return new Promise(r => c.toBlob(b => r(new File([b], 'IMG_2048.jpg', { type: 'image/jpeg' })), 'image/jpeg', 0.9));
  }

  async function play() {
    if (scene === 'call') {
      showCallUI('main');
      CALL.t0 = Date.now() - 84_000;
      setCallState('speaking');
      addCaption('me', 'Hey, what does my afternoon look like?');
      addCaption('bia', 'The design review at 14:30, then the viewing on Carrer de Pujades at 17:00. It is a 20-minute walk, so I will remind you at 16:30.');
      CALL.el.querySelector('.call-time').textContent = fmtDur(84);
    } else if (scene === 'attach') {
      attachDialog([await receipt()]);
      const cap = document.querySelector('.modal textarea');
      if (cap) cap.value = 'Lunch with the client, add it to the November expenses';
    }
  }
})();
