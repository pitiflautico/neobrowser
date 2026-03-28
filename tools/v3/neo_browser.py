"""neo-browser MCP — importable wrapper."""
import runpy, sys, os
sys.path.insert(0, os.path.dirname(__file__))

def main():
    runpy.run_path(os.path.join(os.path.dirname(__file__), 'neo-browser.py'), run_name='__main__')

if __name__ == '__main__':
    main()
