#!/usr/bin/env python3
"""Install the native Cua launcher in an existing owned Blaxel XFCE sandbox."""
import argparse
from fixture import BlaxelTransport, provision, provision_input
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sandbox', required=True)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--input-fixture', action='store_true', help='Also launch the native GTK input/recovery fixture')
    args = parser.parse_args()
    installer = provision_input if args.input_fixture else provision
    installer(BlaxelTransport(args.sandbox, args.workspace))
    print('Installed Cua Driver 0.34.0 and launched native Galculator')
