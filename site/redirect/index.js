// toad.team and toad.computer answer permanently from hotline.dev. The path is
// kept, so a link anyone saved still lands on the page it named; toad.computer
// had one subject, so it arrives at /computer, which the site sends on to the
// page about computers.
const HOME = 'https://hotline.dev';

export default {
  fetch(request) {
    const url = new URL(request.url);
    const prefix = url.hostname.endsWith('toad.computer') ? '/computer' : '';
    const path = url.pathname === '/' ? '' : url.pathname;
    return Response.redirect(HOME + prefix + path + url.search, 301);
  },
};
