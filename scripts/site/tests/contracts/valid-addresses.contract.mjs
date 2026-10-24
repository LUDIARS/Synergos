export default {
  post: (checked, addresses) => checked === addresses.length && checked >= 6
    || 'all valid address spellings must pass the preservation assertions',
};
