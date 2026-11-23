// A pipe is a live cell separator when the run of backslashes immediately before it has
// even length: each pair is one escaped backslash, which leaves the pipe unescaped.
function livePipes(s) {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    if (s[i] !== '|') continue;
    let run = 0;
    for (let j = i - 1; j >= 0 && s[j] === '\\'; j--) run++;
    if (run % 2 === 0) n++;
  }
  return n;
}

const current = (s) => String(s).replace(/\|/g, '\\|');
const fixed = (s) => String(s).replace(/\\/g, '\\\\').replace(/\|/g, '\\|');

const BS = '\\';
const cases = [
  ['plain title', 'plain title'],
  ['bare pipe', 'a|b'],
  ['backslash then pipe', 'a' + BS + '|b'],
  ['two backslashes then pipe', 'a' + BS + BS + '|b'],
  ['trailing backslash', 'ends with ' + BS],
];

let broken = 0;
for (const [name, input] of cases) {
  const c = current(input);
  const f = fixed(input);
  const cLive = livePipes(c);
  const fLive = livePipes(f);
  if (cLive > 0) broken++;
  console.log(name);
  console.log('  input       ', JSON.stringify(input));
  console.log('  current  -> ', JSON.stringify(c), 'live pipes:', cLive, cLive ? 'TABLE BROKEN' : 'ok');
  console.log('  fixed    -> ', JSON.stringify(f), 'live pipes:', fLive, fLive ? 'TABLE BROKEN' : 'ok');
}
console.log('\ncases the current escaping breaks:', broken, 'of', cases.length);
