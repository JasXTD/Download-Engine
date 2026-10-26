// The Rust fixture exports a capture, then its source HTTP server is stopped.
// This opens that actual output in fresh Chrome/Edge profiles with networking off.
// Set IDM_EX_OFFLINE_FIXTURE_OUTPUT to the absolute output folder when running:
// cargo test --offline --locked --test offline_website complete_static_copy
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {pathToFileURL} = require('node:url');
const {chromium} = require('playwright');
const folder = path.resolve(process.argv[2] || 'artifacts/offline-website-browser-fixture');
const results = [];
(async()=>{
  assert(fs.existsSync(path.join(folder,'index.html')), 'Run the Rust capture fixture first');
  for(const [name, executablePath] of [['chrome','C:/Program Files/Google/Chrome/Application/chrome.exe'],['edge','C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe']]) {
    const browser = await chromium.launch({executablePath,headless:true});
    try {
      const context = await browser.newContext({offline:true,viewport:{width:1000,height:700}});
      const page = await context.newPage();
      const requests = [];
      page.on('request',request=>{if(/^https?:/.test(request.url())) requests.push(request.url());});
      await page.goto(pathToFileURL(path.join(folder,'index.html')).href);
      await page.locator('#hero').waitFor();
      const state = await page.evaluate(()=>({
        title:document.title,
        color:getComputedStyle(document.body).color,
        headingSize:getComputedStyle(document.querySelector('h1')).fontSize,
        image:document.querySelector('#hero').complete && document.querySelector('#hero').naturalWidth,
        source:document.querySelector('#hero').currentSrc,
        background:getComputedStyle(document.querySelector('.tile')).backgroundImage,
        scripts:document.scripts.length,
        svgAnimations:document.querySelectorAll('svg set,svg animate,svg animateMotion,svg animateTransform,svg discard').length,
        svgLink:document.querySelector('#svg-next').getAttribute('href'),
        vectorSource:document.querySelector('#vector').currentSrc,
        vectorWidth:document.querySelector('#vector').complete && document.querySelector('#vector').naturalWidth,
      }));
      assert.equal(state.title,'Offline fixture');
      assert.equal(state.color,'rgb(12, 34, 56)');
      assert.equal(state.headingSize,'28px');
      assert.equal(state.image,1);
      assert(state.source.startsWith('file:'));
      assert(state.background.includes('file:'));
      assert.equal(state.scripts,0);
      assert.equal(state.svgAnimations,0);
      assert(state.svgLink.endsWith('.html#section') && !state.svgLink.includes(':'));
      assert.equal(state.vectorWidth,16);
      assert(state.vectorSource.startsWith('file:'));
      await page.screenshot({path:path.join(folder,`${name}-offline.png`),fullPage:true});
      await page.locator('#next').click();
      assert.equal(await page.locator('h1').innerText(),'Page 1');
      assert(page.url().startsWith('file:') && page.url().endsWith('#section'));
      await page.locator('#back').click();
      assert.equal(await page.title(),'Offline fixture');
      await page.locator('#full-image').click();
      assert.equal(page.url(),state.source);
      await page.goBack();
      assert.equal(await page.title(),'Offline fixture');
      await page.goto(state.vectorSource);
      const svgFill = await page.locator('#swatch').evaluate(element=>getComputedStyle(element).fill);
      assert.equal(svgFill,'rgb(0, 128, 0)');
      assert.equal(requests.length,0,JSON.stringify(requests));
      results.push({browser:name,passed:true,offline:true,sourceServerStopped:true,state,standaloneSvgFill:svgFill,networkRequests:requests.length,navigation:'home -> next#section -> home -> linked image -> home -> standalone SVG'});
    } finally { await browser.close(); }
  }
  fs.writeFileSync(path.join(folder,'browser-verification.json'),JSON.stringify({results},null,2));
  console.log(JSON.stringify(results,null,2));
})().catch(error=>{console.error(error);process.exitCode=1;});
