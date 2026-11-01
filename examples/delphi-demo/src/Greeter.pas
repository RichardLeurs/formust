unit Greeter;

interface

function Greeting(const Name: string): string;

implementation

uses
  SysUtils;

function Greeting(const Name: string): string;
begin
  Result := Format('Hello, %s!', [Name]);
end;

end.
