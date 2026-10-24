export default {
  post: (checked, addresses) => checked === addresses.length && checked >= 10
    || 'all unspecified address spellings must pass the rejection assertions',
};
