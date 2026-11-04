# python3 and pip in a shell are bia's own venv. See /usr/local/sbin/bia-venv.
if [ -x /home/bia/.venv/bin/python3 ]; then
  VIRTUAL_ENV=/home/bia/.venv
  PATH="$VIRTUAL_ENV/bin:$PATH"
  export VIRTUAL_ENV PATH
fi
